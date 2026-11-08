//! Playbooks at run time (task "Run plans and playbooks", step 2). An
//! agent's prompt lists its playbooks (`domain::playbook::prompt_index`);
//! `skill_list` and `skill_get` fetch them on demand, answered on the host
//! from its `skills/` folder and read-only, like Studio's guides. Editing
//! goes through `repo_service` (the Playbooks tab), never through a run.

use serde_json::{json, Value};

use crate::engine::ToolOutcome;
use crate::llm::ToolSpec;
use crate::services::domain::playbook::{self, Playbook};
use crate::services::repo_store::RepoStore;

pub const LIST: &str = "skill_list";
pub const GET: &str = "skill_get";
const TOOLS: [&str; 2] = [LIST, GET];

pub fn is_playbook_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}

/// Agent `agent_id`'s valid playbooks; broken files are logged and left out
/// so a run never fails on one
pub fn load(store: &RepoStore, agent_id: &str) -> Vec<Playbook> {
    match store.repos().agents.load_playbooks(agent_id) {
        Ok(loaded) => {
            for problem in &loaded.problems {
                log::warn!(
                    "Skipped a playbook of '{agent_id}' ({}): {}",
                    problem
                        .source_path
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                    problem.error
                );
            }
            loaded.entities
        }
        Err(e) => {
            log::warn!("Couldn't read the playbooks of '{agent_id}': {e}");
            Vec::new()
        }
    }
}

/// The prompt section listing `playbooks` (empty without any)
pub fn prompt_section(playbooks: &[Playbook]) -> String {
    playbook::prompt_index(playbooks)
}

/// The two tools, offered to an agent that has playbooks
pub fn specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: LIST.to_string(),
            description: "List your playbooks: each one's name, what it covers and when to use it."
                .to_string(),
            input_schema: json!({ "type": "object", "properties": {} }),
        },
        ToolSpec {
            name: GET.to_string(),
            description: "Read one of your playbooks (step-by-step guides for work you do) by its \
name, before you plan work it covers. Follow it unless the user says otherwise."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["name"],
                "properties": {
                    "name": {"type": "string", "description": "The playbook's name, e.g. `pay-a-bill`"}
                }
            }),
        },
    ]
}

fn outcome(value: Value, is_error: bool) -> ToolOutcome {
    ToolOutcome {
        result: match value {
            Value::String(text) => text,
            other => other.to_string(),
        },
        is_error,
    }
}

/// Answers `tool` from `playbooks` (pure, so it's tested without files)
pub fn answer(playbooks: &[Playbook], tool: &str, args: &Value) -> ToolOutcome {
    match tool {
        LIST => outcome(
            json!(playbooks
                .iter()
                .map(|p| json!({ "name": p.name(), "description": p.description(), "when": p.when() }))
                .collect::<Vec<_>>()),
            false,
        ),
        GET => {
            let name = args.get("name").and_then(Value::as_str).unwrap_or("").trim();
            if name.is_empty() {
                return outcome(json!("name is required"), true);
            }
            match playbooks.iter().find(|p| p.name() == name) {
                Some(p) => outcome(
                    json!(format!("# {}\n{}\n\n{}", p.name(), p.description(), p.body())),
                    false,
                ),
                None => outcome(
                    json!(format!(
                        "You have no playbook '{name}'. Yours: {}.",
                        if playbooks.is_empty() {
                            "none".to_string()
                        } else {
                            playbooks.iter().map(Playbook::name).collect::<Vec<_>>().join(", ")
                        }
                    )),
                    true,
                ),
            }
        }
        other => outcome(json!(format!("Unknown playbook tool '{other}'")), true),
    }
}

/// Answers a playbook tool call for agent `agent_id`, from its files now
pub fn call(store: &RepoStore, agent_id: &str, tool: &str, args: &Value) -> ToolOutcome {
    answer(&load(store, agent_id), tool, args)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playbooks() -> Vec<Playbook> {
        vec![
            Playbook::new(
                "pay-a-bill",
                "Pay a bill online",
                "A bill is due",
                "1. Plan.",
            )
            .unwrap(),
            Playbook::new(
                "web-task",
                "Any website work",
                "Work needs a site",
                "1. Open.",
            )
            .unwrap(),
        ]
    }

    #[test]
    fn lists_the_index_without_bodies() {
        let listed = answer(&playbooks(), LIST, &json!({}));
        assert!(!listed.is_error);
        let value: Value = serde_json::from_str(&listed.result).unwrap();
        assert_eq!(value[0]["name"], "pay-a-bill");
        assert_eq!(value[1]["when"], "Work needs a site");
        assert!(!listed.result.contains("1. Plan."));
    }

    #[test]
    fn gets_a_body_by_name() {
        let got = answer(&playbooks(), GET, &json!({ "name": " pay-a-bill " }));
        assert!(!got.is_error);
        assert_eq!(got.result, "# pay-a-bill\nPay a bill online\n\n1. Plan.");
        let missing = answer(&playbooks(), GET, &json!({ "name": "nope" }));
        assert!(missing.is_error);
        assert!(missing.result.contains("pay-a-bill, web-task"));
        assert!(answer(&playbooks(), GET, &json!({})).is_error);
        assert!(answer(&[], GET, &json!({ "name": "x" }))
            .result
            .contains("none"));
    }

    #[test]
    fn the_tools_are_known_by_name() {
        assert!(is_playbook_tool(LIST) && is_playbook_tool(GET));
        assert!(!is_playbook_tool("plan_set"));
        let names: Vec<String> = specs().into_iter().map(|s| s.name).collect();
        assert_eq!(names, [LIST, GET]);
    }

    #[test]
    fn reads_an_agents_files() {
        let store = RepoStore::for_tests(crate::db::Db::in_memory());
        crate::services::builtin_agents::seed(store.layout()).unwrap();
        let chief = crate::services::builtin_agents::CHIEF_OF_STAFF;
        let loaded = load(&store, chief);
        assert!(loaded.iter().any(|p| p.name() == "pay-a-bill"));
        let got = call(&store, chief, GET, &json!({ "name": "pay-a-bill" }));
        assert!(!got.is_error, "{}", got.result);
        assert!(prompt_section(&loaded).contains("`pay-a-bill`"));
        assert!(load(&store, "nobody").is_empty());
    }
}
