//! Playbooks at run time (task "Run plans and playbooks", step 2). An
//! agent's prompt lists its playbooks (`domain::playbook::prompt_index`);
//! `skill_list` and `skill_get` fetch them on demand, answered on the host
//! from its `skills/` folder and read-only, like Studio's guides.
//!
//! Writing goes through `repo_service`: the Playbooks tab, or `skill_propose`
//! (step 5, "remember how I do this"). A proposal is checked by the domain
//! rules before the user sees it, always waits for their approval (the card
//! shows the playbook and what it replaces), and only then is written and
//! committed. Agents installed from a pack can't propose: their playbooks
//! are read-only.

use serde_json::{json, Map, Value};

use crate::db::PlannedCall;
use crate::engine::ToolOutcome;
use crate::llm::ToolSpec;
use crate::services::domain::playbook::{self, Playbook};
use crate::services::domain::AgentId;
use crate::services::repo_service::PlaybookInput;
use crate::services::repo_store::RepoStore;

pub const LIST: &str = "skill_list";
pub const GET: &str = "skill_get";
/// Drafts a playbook (new, or a change to one) for the user to approve
pub const PROPOSE: &str = "skill_propose";
const TOOLS: [&str; 2] = [LIST, GET];
/// The key the approval card reads for the playbook a proposal replaces.
/// Nandi sets it from the files; the model's own value is dropped.
pub const CURRENT: &str = "current";
/// What the prompt tells an agent that can propose playbooks
const PROPOSE_HINT: &str = "When the user tells you how they want something done (\"remember: …\"), draft a playbook and propose it (`skill_propose`): it's saved only if they approve.";

/// The read-only tools (`skill_list`, `skill_get`)
pub fn is_playbook_tool(name: &str) -> bool {
    TOOLS.contains(&name)
}

pub fn is_propose(name: &str) -> bool {
    name == PROPOSE
}

/// Whether agent `agent_id` may propose playbooks: one made on this
/// computer (the Chief of Staff too), not one installed from a pack
pub fn can_propose(store: &RepoStore, agent_id: &str) -> bool {
    store
        .repos()
        .agents
        .locate(agent_id)
        .is_some_and(|place| !place.origin.is_vendored())
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

/// The prompt section listing `playbooks`, and how to propose one when the
/// agent can. Empty when there's neither.
pub fn prompt_section(playbooks: &[Playbook], can_propose: bool) -> String {
    match (playbooks.is_empty(), can_propose) {
        (true, false) => String::new(),
        (true, true) => format!("\n\n## Your playbooks\nYou have none yet. {PROPOSE_HINT}\n"),
        (false, false) => playbook::prompt_index(playbooks),
        (false, true) => format!("{}{PROPOSE_HINT}\n", playbook::prompt_index(playbooks)),
    }
}

/// `skill_propose`, offered to an agent that [`can_propose`]
pub fn propose_spec() -> ToolSpec {
    let line = |what: &str| json!({ "type": "string", "description": what });
    ToolSpec {
        name: PROPOSE.to_string(),
        description: "Propose a playbook: a step-by-step guide for how the user wants some work \
done, written from what they told you or from work you repeat. The user sees it and approves or \
rejects it; nothing is saved before. Reuse a playbook's name to change it (the user sees what it \
replaces). Say in your reply that it waits for their approval."
            .to_string(),
        input_schema: json!({
            "type": "object",
            "required": ["name", "description", "when", "body", "reason"],
            "properties": {
                "name": line("kebab-case, e.g. `pay-bills`"),
                "description": line("What it covers, one line"),
                "when": line("When to use it, one line, e.g. `A bill is due`"),
                "body": line("The steps in Markdown, 900 words or fewer, in the user's terms (\"Pay from Chase, never the card\")"),
                "reason": line("One line for the user: why you propose it")
            }
        }),
    }
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

/// The playbook a `skill_propose` call would write, checked by the domain
/// rules (name, one-line fields, 900 words, no conflict markers), with a
/// reason for the user. Pure.
pub fn proposed(args: &Value) -> Result<Playbook, String> {
    let text = |key: &str| args.get(key).and_then(Value::as_str).unwrap_or("");
    let playbook = Playbook::new(
        text("name"),
        text("description"),
        text("when"),
        text("body"),
    )
    .map_err(|e| e.to_string())?;
    if text("reason").trim().is_empty() {
        return Err("reason is required: one line telling the user why".to_string());
    }
    Ok(playbook)
}

/// `args` with [`CURRENT`]: the playbook of that name it would replace,
/// from `playbooks`, never the model's own value. Pure.
pub fn with_current(args: Value, playbooks: &[Playbook]) -> Value {
    let Value::Object(mut args) = args else {
        return args;
    };
    args.remove(CURRENT);
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if let Some(current) = playbooks.iter().find(|p| p.name() == name) {
        let mut fields = Map::new();
        fields.insert("description".into(), json!(current.description()));
        fields.insert("when".into(), json!(current.when()));
        fields.insert("body".into(), json!(current.body()));
        args.insert(CURRENT.into(), Value::Object(fields));
    }
    Value::Object(args)
}

/// Every `skill_propose` call waits for the user, and its card shows the
/// playbook it replaces. For both engines, once the step is planned.
pub fn gate(store: &RepoStore, agent_id: &str, calls: &mut [PlannedCall]) {
    if !calls.iter().any(|c| is_propose(&c.tool)) {
        return;
    }
    let current = load(store, agent_id);
    for call in calls.iter_mut().filter(|c| is_propose(&c.tool)) {
        call.requires_approval = true;
        call.args = with_current(std::mem::take(&mut call.args), &current);
    }
}

/// The playbook agent `agent_id` proposes, or why it can't: checked before
/// the user is asked, and again before writing
pub fn check(store: &RepoStore, agent_id: &str, args: &Value) -> Result<Playbook, String> {
    if !can_propose(store, agent_id) {
        return Err(
            "Your playbooks came with your pack and are read-only: tell the user what you'd \
change instead."
                .to_string(),
        );
    }
    let playbook = proposed(args)?;
    // A broken file under that name is still someone's text
    let broken = store
        .repos()
        .agents
        .load_playbooks(agent_id)
        .map(|loaded| {
            loaded.problems.iter().any(|problem| {
                problem
                    .source_path
                    .as_deref()
                    .and_then(std::path::Path::file_stem)
                    .is_some_and(|stem| stem == playbook.name())
            })
        })
        .unwrap_or(false);
    if broken {
        return Err(format!(
            "skills/{}.md can't be read, so it can't be replaced: choose another name, or ask \
the user to fix it on the Playbooks tab.",
            playbook.name()
        ));
    }
    Ok(playbook)
}

/// Writes an approved proposal into the agent's `skills/` and commits. A
/// playbook of that name is replaced.
pub fn apply(store: &RepoStore, agent_id: &str, args: &Value) -> ToolOutcome {
    let playbook = match check(store, agent_id, args) {
        Ok(playbook) => playbook,
        Err(why) => return outcome(json!(why), true),
    };
    let id = match AgentId::parse(agent_id) {
        Ok(id) => id,
        Err(e) => return outcome(json!(e.to_string()), true),
    };
    let replaces = load(store, agent_id)
        .iter()
        .any(|p| p.name() == playbook.name());
    let input = PlaybookInput {
        name: playbook.name().to_string(),
        description: playbook.description().to_string(),
        when: playbook.when().to_string(),
        body: playbook.body().to_string(),
    };
    match store.save_agent_playbook(&id, &input, replaces.then_some(playbook.name())) {
        Ok(saved) => outcome(
            json!(format!(
                "The user approved: playbook '{}' {}. It's in your playbooks from now on.",
                saved.name(),
                if replaces { "updated" } else { "saved" }
            )),
            false,
        ),
        Err(e) => outcome(json!(format!("Couldn't save the playbook: {e}")), true),
    }
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
        assert!(prompt_section(&loaded, false).contains("`pay-a-bill`"));
        assert!(load(&store, "nobody").is_empty());
    }

    fn proposal(name: &str, body: &str) -> Value {
        json!({
            "name": name,
            "description": "Pay the household bills",
            "when": "A bill is due",
            "body": body,
            "reason": "You told me how you pay bills"
        })
    }

    #[test]
    fn a_proposal_is_checked_by_the_domain_rules() {
        let ok = proposed(&proposal(
            " pay-bills ",
            "1. Pay from Chase, never the card.",
        ))
        .unwrap();
        assert_eq!(ok.name(), "pay-bills");
        assert_eq!(ok.body(), "1. Pay from Chase, never the card.");
        assert!(proposed(&proposal("Pay Bills", "1. Pay."))
            .unwrap_err()
            .contains("kebab-case"));
        assert!(proposed(&proposal("pay-bills", "  ")).is_err());
        let long = "word ".repeat(playbook::MAX_PLAYBOOK_WORDS + 1);
        assert!(proposed(&proposal("pay-bills", &long))
            .unwrap_err()
            .contains("words"));
        let mut no_reason = proposal("pay-bills", "1. Pay.");
        no_reason["reason"] = json!(" ");
        assert!(proposed(&no_reason).unwrap_err().contains("reason"));
        let mut two_lines = proposal("pay-bills", "1. Pay.");
        two_lines["when"] = json!("A bill\nis due");
        assert!(proposed(&two_lines).unwrap_err().contains("one line"));
    }

    #[test]
    fn the_card_sees_what_a_proposal_replaces_from_the_files() {
        let mut faked = proposal("pay-a-bill", "1. New.");
        faked[CURRENT] = json!({ "body": "made up" });
        let args = with_current(faked, &playbooks());
        assert_eq!(args[CURRENT]["body"], "1. Plan.");
        assert_eq!(args[CURRENT]["when"], "A bill is due");
        // A new name replaces nothing, whatever the model says
        let mut faked = proposal("pay-bills", "1. New.");
        faked[CURRENT] = json!({ "body": "made up" });
        assert!(with_current(faked, &playbooks()).get(CURRENT).is_none());
        assert_eq!(with_current(json!("x"), &playbooks()), json!("x"));
    }

    #[test]
    fn the_prompt_says_how_to_propose_when_the_agent_can() {
        assert_eq!(prompt_section(&[], false), "");
        let none_yet = prompt_section(&[], true);
        assert!(none_yet.contains("none yet") && none_yet.contains("`skill_propose`"));
        let both = prompt_section(&playbooks(), true);
        assert!(both.contains("`pay-a-bill`") && both.contains("`skill_propose`"));
        assert!(!prompt_section(&playbooks(), false).contains("skill_propose"));
        assert_eq!(propose_spec().name, PROPOSE);
        assert!(is_propose(PROPOSE) && !is_playbook_tool(PROPOSE));
    }

    fn seeded() -> RepoStore {
        let store = RepoStore::for_tests(crate::db::Db::in_memory());
        crate::services::builtin_agents::seed(store.layout()).unwrap();
        store
    }

    #[test]
    fn every_proposal_waits_for_the_user() {
        let store = seeded();
        let chief = crate::services::builtin_agents::CHIEF_OF_STAFF;
        let mut calls = vec![
            PlannedCall {
                id: "c1".into(),
                tool: PROPOSE.into(),
                args: proposal("pay-a-bill", "1. Pay from Chase."),
                requires_approval: false,
            },
            PlannedCall {
                id: "c2".into(),
                tool: GET.into(),
                args: json!({ "name": "pay-a-bill" }),
                requires_approval: false,
            },
        ];
        gate(&store, chief, &mut calls);
        assert!(calls[0].requires_approval);
        assert!(calls[0].args[CURRENT]["body"]
            .as_str()
            .unwrap()
            .contains("hand"));
        assert!(!calls[1].requires_approval);
    }

    #[test]
    fn an_approved_proposal_is_written_and_committed() {
        let store = seeded();
        let chief = crate::services::builtin_agents::CHIEF_OF_STAFF;
        let saved = apply(&store, chief, &proposal("pay-bills", "1. Pay from Chase."));
        assert!(!saved.is_error, "{}", saved.result);
        assert!(saved.result.contains("'pay-bills' saved"));
        let body = |name: &str| {
            load(&store, chief)
                .into_iter()
                .find(|p| p.name() == name)
                .map(|p| p.body().to_string())
        };
        assert_eq!(body("pay-bills").as_deref(), Some("1. Pay from Chase."));
        // The same name replaces it
        let updated = apply(
            &store,
            chief,
            &proposal("pay-bills", "1. Pay from Chase, never the card."),
        );
        assert!(updated.result.contains("updated"), "{}", updated.result);
        assert_eq!(
            body("pay-bills").as_deref(),
            Some("1. Pay from Chase, never the card.")
        );
        assert_eq!(
            load(&store, chief)
                .iter()
                .filter(|p| p.name() == "pay-bills")
                .count(),
            1
        );
        // Invalid: nothing written
        assert!(apply(&store, chief, &proposal("Bad Name", "x")).is_error);
        assert!(body("bad-name").is_none());
    }

    #[test]
    fn a_broken_file_of_that_name_is_never_overwritten() {
        let store = seeded();
        let chief = crate::services::builtin_agents::CHIEF_OF_STAFF;
        let dir = store.repos().agents.playbooks_dir(chief).unwrap();
        std::fs::write(dir.join("pay-bills.md"), "no frontmatter").unwrap();
        let refused = check(&store, chief, &proposal("pay-bills", "1. Pay."));
        assert!(refused.unwrap_err().contains("can't be read"));
        assert_eq!(
            std::fs::read_to_string(dir.join("pay-bills.md")).unwrap(),
            "no frontmatter"
        );
    }

    #[test]
    fn an_installed_agent_cant_propose() {
        let store = seeded();
        let dir = store.layout().vendored_agent_dir("alice", "arxiv");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("skill.md"), "---\nname: ArXiv\n---\nSearch.").unwrap();
        assert!(!can_propose(&store, "arxiv"));
        assert!(!can_propose(&store, "nobody"));
        assert!(can_propose(
            &store,
            crate::services::builtin_agents::CHIEF_OF_STAFF
        ));
        let refused = apply(&store, "arxiv", &proposal("search", "1. Search."));
        assert!(refused.is_error);
        assert!(refused.result.contains("read-only"));
        assert!(!dir.join("skills/search.md").exists());
    }
}
