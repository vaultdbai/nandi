//! Checks a proposed or edited artifact the way saving would, plus what the
//! domain parser leaves alone: tool IDs that don't exist, a model whose
//! provider has no key, a CLI runtime without its model, references to
//! agents that aren't there. Parse failures are errors (Save and Apply
//! stay off); the rest are warnings. Pure: the facts come from the caller.

use std::collections::BTreeSet;

use serde::Serialize;
use specta::Type;

use crate::services::ask::ArtifactRef;
use crate::services::connectors::{self, folders};
use crate::services::domain::{self, AgentId, AgentRuntimeKind};
use crate::services::gateway::manifest;
use crate::services::repos::capability_repo;

/// What a check found
#[derive(Debug, Clone, Default, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactValidation {
    /// No errors (warnings allowed)
    pub ok: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// What exists on this Nandi, for the checks the parser can't do
#[derive(Debug, Clone, Default)]
pub struct ValidationFacts {
    /// Tool IDs in the catalog (built-in, connector, pilot)
    pub tool_ids: BTreeSet<String>,
    /// Model prefixes calls can use now: providers with a key, plus `ollama`
    pub usable_providers: BTreeSet<String>,
    pub agents: BTreeSet<String>,
    /// Installed connector IDs
    pub connectors: BTreeSet<String>,
    /// Shared folder IDs, by filesystem connector
    pub folders: std::collections::BTreeMap<String, Vec<String>>,
}

/// Claude Code's tool names, which Nandi doesn't have, and Nandi's nearest
const FOREIGN_TOOLS: &[(&str, &str)] = &[
    ("Read", "read_file"),
    ("Write", "write_file"),
    ("Edit", "write_file"),
    ("MultiEdit", "write_file"),
    ("Bash", "shell"),
    ("Glob", "list_dir"),
    ("Grep", "shell"),
    ("LS", "list_dir"),
    ("WebFetch", "shell"),
    ("Task", "delegate_to_agent"),
];

pub fn validate(
    artifact: &ArtifactRef,
    content: &str,
    facts: &ValidationFacts,
) -> ArtifactValidation {
    let result = match artifact {
        ArtifactRef::Agent(id) => return validate_agent(id, content, facts),
        ArtifactRef::Workflow(_) => return validate_workflow(content, facts),
        ArtifactRef::Capability(id) => capability_repo::load_text(content, id)
            .map(|_| ())
            .map_err(|e| format!("Not a valid capability: {e}")),
        // The parser the app enforces with: Studio can't save a policy
        // that would block agents
        ArtifactRef::Policy(id) => crate::services::domain::policy::Policy::parse(id, content)
            .map(|_| ())
            .map_err(|e| format!("Not a valid policy: {e}")),
        ArtifactRef::Gateway => manifest::parse(content)
            .map(|_| ())
            .map_err(|e| format!("Not a valid sharing file: {e}")),
    };
    match result {
        Ok(()) => ArtifactValidation {
            ok: true,
            ..Default::default()
        },
        Err(error) => failed(error),
    }
}

fn failed(error: String) -> ArtifactValidation {
    ArtifactValidation {
        ok: false,
        errors: vec![error],
        warnings: Vec::new(),
    }
}

pub fn validate_agent(id: &str, content: &str, facts: &ValidationFacts) -> ArtifactValidation {
    let id = match AgentId::parse(id) {
        Ok(id) => id,
        Err(e) => return failed(format!("Not a valid agent ID: {e}")),
    };
    let agent = match domain::Agent::from_skill_md(id, content) {
        Ok(agent) => agent,
        Err(e) => return failed(format!("Not a valid agent: {e}")),
    };
    let mut warnings = Vec::new();

    let tools = agent.capabilities().tools();
    let named: Vec<&String> = tools
        .allowed()
        .unwrap_or_default()
        .iter()
        .chain(tools.declared_approvals().unwrap_or_default())
        .collect();
    let mut seen = BTreeSet::new();
    for tool in named {
        if !seen.insert(tool.as_str()) || tool_exists(tool, facts) {
            continue;
        }
        warnings.push(match FOREIGN_TOOLS.iter().find(|(name, _)| name == tool) {
            Some((_, nandi)) => format!(
                "'{tool}' is a Claude Code tool name, not a Nandi tool; use '{nandi}'. It will be ignored."
            ),
            None => format!("Tool '{tool}' doesn't exist in Nandi; it will be ignored."),
        });
    }

    let mut errors = Vec::new();
    let options = connectors::declared_options(&agent);
    for connector in connectors::declared_connectors(&agent) {
        if !facts.connectors.contains(&connector) {
            warnings.push(format!(
                "The connector '{connector}' isn't installed, so its tools are unavailable: install it first (Library › Connectors, or save it with kind connector)."
            ));
            continue;
        }
        let Some(list) = facts.folders.get(&connector) else {
            continue;
        };
        match folders::grants(options.get(&connector)) {
            Err(e) => errors.push(format!("connectors › {connector}: {e}")),
            Ok(grants) => {
                for grant in grants.iter().filter(|g| !list.contains(&g.id)) {
                    errors.push(format!(
                        "There's no shared folder '{}'. Shared folders: {}. Add it in Library › Connectors › {connector} first.",
                        grant.id,
                        if list.is_empty() { "none yet".to_string() } else { list.join(", ") }
                    ));
                }
            }
        }
    }

    if let Some(model) = agent.model() {
        let prefix = model.provider_kind().prefix();
        if !facts.usable_providers.contains(prefix) {
            warnings.push(format!(
                "There's no key for '{prefix}' models yet, so this agent will use another model until you add one in Settings › Models & runtimes."
            ));
        }
    }

    if !matches!(agent.runtime(), AgentRuntimeKind::Native)
        && agent
            .runtime_config()
            .and_then(|config| config.model.as_deref())
            .is_none_or(|model| model.trim().is_empty())
    {
        warnings.push(format!(
            "The {} runtime uses its own default model; set runtime_config.model to choose one.",
            agent.runtime().as_str()
        ));
    }

    ArtifactValidation {
        ok: errors.is_empty(),
        errors,
        warnings,
    }
}

/// Catalog tools, plus connector tools (`<connector>__<tool>`) the catalog
/// may not list until the connector is installed
fn tool_exists(tool: &str, facts: &ValidationFacts) -> bool {
    facts.tool_ids.contains(tool) || tool.contains("__")
}

pub fn validate_workflow(content: &str, facts: &ValidationFacts) -> ArtifactValidation {
    let workflow = match domain::Workflow::from_yaml(content) {
        Ok(workflow) => workflow,
        Err(e) => return failed(format!("Not a valid workflow: {e}")),
    };
    let warnings = workflow
        .steps()
        .iter()
        .filter_map(|step| step.agent.as_ref())
        .filter(|agent| !facts.agents.contains(agent.as_str()))
        .map(|agent| {
            format!("No agent '{agent}' on this Nandi; that step will fail until it exists.")
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    ArtifactValidation {
        ok: true,
        errors: Vec::new(),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> ValidationFacts {
        ValidationFacts {
            tool_ids: [
                "shell",
                "read_file",
                "write_file",
                "list_dir",
                "delegate_to_agent",
            ]
            .map(String::from)
            .into(),
            usable_providers: ["claude", "ollama"].map(String::from).into(),
            agents: ["writer".to_string()].into(),
            connectors: ["filesystem".to_string()].into(),
            folders: [(
                "filesystem".to_string(),
                vec!["docs".to_string(), "reports".to_string()],
            )]
            .into(),
        }
    }

    #[test]
    fn folders_must_be_on_the_list_and_connectors_installed() {
        let skill =
            |connectors: &str| format!("---\nname: A\nconnectors: {connectors}\n---\nGo.\n");
        let good = validate_agent(
            "a",
            &skill("[{id: filesystem, folders: [docs, {id: reports, access: read}]}]"),
            &facts(),
        );
        assert!(good.ok && good.warnings.is_empty(), "{good:?}");

        let unknown = validate_agent(
            "a",
            &skill("[{id: filesystem, folders: [photos]}]"),
            &facts(),
        );
        assert!(!unknown.ok);
        assert!(unknown.errors[0].contains("no shared folder 'photos'"));
        assert!(unknown.errors[0].contains("docs, reports"));

        let bad = validate_agent("a", &skill("[{id: filesystem, folders: docs}]"), &facts());
        assert!(!bad.ok);

        let missing = validate_agent("a", &skill("[web_search]"), &facts());
        assert!(missing.ok);
        assert!(missing.warnings[0].contains("isn't installed"));
    }

    /// The proposal from review shot 27, which Nandi can't load
    const SHOT_27: &str = "---\nname: Run Summarizer\nmodel: claude-sonnet-4-5\n\
        allowed-tools: [Read, Glob, Grep]\n---\nLook in ~/.nandi/agents/*/runs.\n";

    #[test]
    fn the_shot_27_proposal_is_rejected() {
        let result = validate_agent("run-summarizer", SHOT_27, &facts());
        assert!(!result.ok);
        assert!(
            result.errors[0].contains("<provider>:<model>"),
            "{result:?}"
        );
    }

    #[test]
    fn claude_code_tool_names_are_flagged_with_the_nandi_name() {
        let content = "---\nname: R\nmodel: claude:claude-sonnet-5\n\
            allowed-tools: [Read, Glob, shell, made_up]\n---\nBody\n";
        let result = validate_agent("r", content, &facts());
        assert!(result.ok);
        assert_eq!(result.warnings.len(), 3, "{result:?}");
        assert!(
            result.warnings[0].contains("'Read' is a Claude Code tool")
                && result.warnings[0].contains("'read_file'")
        );
        assert!(result.warnings[1].contains("'list_dir'"));
        assert!(result.warnings[2].contains("Tool 'made_up' doesn't exist"));
    }

    #[test]
    fn a_model_without_a_key_is_a_warning() {
        let content = "---\nname: R\nmodel: gpt:gpt-5\n---\nBody\n";
        let result = validate_agent("r", content, &facts());
        assert!(result.ok);
        assert!(
            result.warnings[0].contains("no key for 'gpt'"),
            "{result:?}"
        );
        // Nothing to say about a model that can be called
        let fine = "---\nname: R\nmodel: claude:claude-sonnet-5\nallowed-tools: [shell]\n---\nB\n";
        assert_eq!(
            validate_agent("r", fine, &facts()),
            ArtifactValidation {
                ok: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn a_cli_runtime_without_its_model_is_a_warning() {
        let content = "---\nname: R\nruntime: claude-code\n---\nBody\n";
        let result = validate_agent("r", content, &facts());
        assert!(
            result.warnings[0].contains("runtime_config.model"),
            "{result:?}"
        );
        let with_model =
            "---\nname: R\nruntime: claude-code\nruntime_config: {model: sonnet}\n---\nBody\n";
        assert!(validate_agent("r", with_model, &facts())
            .warnings
            .is_empty());
    }

    #[test]
    fn workflows_naming_missing_agents_get_a_warning() {
        let yaml = "id: daily\nname: Daily\nsteps:\n  - {id: a, agent: writer, input: go}\n  - {id: b, agent: ghost, input: '{{a.output}}'}\n";
        let result = validate_workflow(yaml, &facts());
        assert!(result.ok, "{result:?}");
        assert_eq!(
            result.warnings,
            ["No agent 'ghost' on this Nandi; that step will fail until it exists."]
        );
        assert!(!validate_workflow("id: [", &facts()).ok);
    }

    #[test]
    fn other_kinds_are_parsed() {
        assert!(!validate(&ArtifactRef::Gateway, "---\n[\n---\n", &facts()).ok);
        assert!(
            validate(&ArtifactRef::Agent("bad id".into()), "", &facts()).errors[0]
                .contains("agent ID")
        );
    }
}
