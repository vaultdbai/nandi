//! Studio's system prompt, built every turn from the code it describes: the
//! skill.md schema from the domain (`SKILL_FRONTMATTER_KEYS`), model
//! prefixes from `Provider`, tool IDs from the catalog, and the models this
//! Nandi can call. Nothing about Nandi's formats is written here by hand, so
//! the prompt can't drift from what saving accepts. Pure.

use crate::services::domain::agent::{Provider, SKILL_FRONTMATTER_KEYS};

use super::guides;
use super::tools::{
    CHECK_ARTIFACT, GET_AGENT, GET_GUIDE, GET_REQUEST, GET_WORKFLOW, LIST_RUNS, SAVE_ARTIFACT,
    START_REQUEST,
};
use crate::services::ask::ArtifactRef;

/// What changes from turn to turn
#[derive(Debug, Clone, Default)]
pub struct PromptFacts {
    /// Tool IDs in the catalog, in catalog order
    pub tool_ids: Vec<String>,
    /// The model new agents should use (the user's default, if callable)
    pub default_model: String,
    /// `<provider>:<model>` examples the user has keys for
    pub usable_models: Vec<String>,
    /// People on the network who take requests (`a2a::tool`)
    pub network_section: String,
    /// The turn is the companion's conversation (`services::companion`)
    pub companion: Option<CompanionFacts>,
    /// The file open in Studio's editor
    pub open_artifact: Option<ArtifactRef>,
}

/// What the prompt says about the file open in the editor
fn open_artifact_section(artifact: &ArtifactRef) -> String {
    let (what, read) = match artifact {
        ArtifactRef::Agent(id) => (format!("the agent `{id}`"), Some(GET_AGENT)),
        ArtifactRef::Workflow(id) => (format!("the workflow `{id}`"), Some(GET_WORKFLOW)),
        ArtifactRef::Capability(id) => (format!("the capability `{id}`"), None),
        ArtifactRef::Policy(id) => (format!("the policy `{id}`"), None),
        ArtifactRef::Gateway => ("NANDI.md".to_string(), None),
    };
    let read = match read {
        Some(tool) => format!(" Read it with `{tool}` before you answer about it."),
        None => String::new(),
    };
    format!(
        "\n## Open in the editor\n\
The user has {what} open in Ask's editor: \"this\" and \"it\" mean that file.{read}\n"
    )
}

/// What the companion adds to Studio's prompt
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CompanionFacts {
    /// The Request the conversation is pinned to
    pub request_id: Option<String>,
}

/// How a Request is linked in a reply, so the companion renders it as a
/// live card (`src/components/companion/RequestLink.tsx`)
pub const REQUEST_LINK_PREFIX: &str = "nandi://request/";

fn companion_section(companion: &CompanionFacts) -> String {
    let scope = match &companion.request_id {
        Some(id) => format!(
            "\nThis conversation is about the Request `{id}`: call `{GET_REQUEST}` with that ID before \
answering, and read \"it\" or \"this\" as that Request."
        ),
        None => String::new(),
    };
    format!(
        "\n## In the companion\n\
You're answering in the companion, a small window over the user's other apps. Keep replies short. \
When you mention a Request, link it as `[its title]({REQUEST_LINK_PREFIX}<id>)` so it shows as a live \
card. Don't show or edit agent, workflow or trigger files here: say what you'd change and that it opens \
in Ask (the companion has an Open in Ask button).{scope}\n"
    )
}

/// Where definitions live, relative to the user's home
pub const REPO_LAYOUT: &[(&str, &str)] = &[
    ("~/.nandi/repo/.nandi/agents/<id>/skill.md", "an agent"),
    (
        "~/.nandi/repo/.nandi/agents/<id>/instructions.md",
        "optional longer instructions",
    ),
    ("~/.nandi/repo/.nandi/workflows/<id>.yaml", "a workflow"),
    ("~/.nandi/repo/.nandi/triggers/<id>.yaml", "a trigger"),
    ("~/.nandi/repo/.nandi/settings.yaml", "portable settings"),
];

pub fn studio_prompt(facts: &PromptFacts) -> String {
    let layout = REPO_LAYOUT
        .iter()
        .map(|(path, what)| format!("- `{path}`: {what}"))
        .collect::<Vec<_>>()
        .join("\n");
    let schema = SKILL_FRONTMATTER_KEYS
        .iter()
        .map(|(key, meaning)| format!("- `{key}`: {meaning}"))
        .collect::<Vec<_>>()
        .join("\n");
    let prefixes = Provider::ALL
        .iter()
        .map(|p| format!("`{}`", p.prefix()))
        .collect::<Vec<_>>()
        .join(", ");
    let tools = facts
        .tool_ids
        .iter()
        .map(|id| format!("`{id}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let usable = if facts.usable_models.is_empty() {
        "No model key is set yet; tell the user to add one in Settings › Models & runtimes."
            .to_string()
    } else {
        format!(
            "Models this Nandi can call now: {}.",
            facts
                .usable_models
                .iter()
                .map(|m| format!("`{m}`"))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let default_model = &facts.default_model;
    let companion = facts
        .companion
        .as_ref()
        .map(companion_section)
        .unwrap_or_default();
    let open = facts
        .open_artifact
        .as_ref()
        .map(open_artifact_section)
        .unwrap_or_default();

    format!(
        "You are Nandi, the assistant on the Ask page of Nandi, a desktop app for running AI agents.\n\
\n\
## Answer from facts\n\
You have read-only tools that look at this Nandi: runs, requests, agents, workflows, triggers, tools, \
models and the network. Call `{LIST_RUNS}` before answering anything about what happened or what agents did; \
never say you can't see runs or history. If a tool finds nothing, say so plainly (\"No runs today\") and \
suggest a next step.\n\
\n\
To get something done (\"run X\", \"do Y for me\", \"every Monday…\"), read the matching guide, then use \
`{START_REQUEST}`: the user approves its plan before anything starts. Use `delegate_to_agent` only to ask \
one person or agent directly.\n\
\n\
## How to do things\n\
Before you propose an agent, workflow or trigger, or explain a feature, read the matching guide with \
`{GET_GUIDE}`:\n\
{guide_index}\n\
\n\
## Where things live\n\
Definitions are files in the user's repo. The user doesn't edit them by path: you write them with \
`{SAVE_ARTIFACT}`, which creates or replaces agents, workflows, triggers, connectors (tools from MCP \
servers), capabilities and policies.\n\
{layout}\n\
\n\
## skill.md\n\
A skill.md is YAML frontmatter between `---` lines, then the agent's instructions in Markdown. \
These are the only frontmatter keys:\n\
{schema}\n\
\n\
## Models\n\
A model is written `<provider>:<model>`, with provider one of {prefixes}; for example \
`claude:claude-sonnet-5`. A bare name like `claude-sonnet-4-5` is invalid. {usable} \
Give new agents `model: {default_model}` unless the user asks for another. Agents on a CLI runtime \
(`runtime: claude-code`) set that CLI's model in `runtime_config.model` instead.\n\
\n\
## Tools agents can use\n\
Tool IDs in this Nandi: {tools}. Use only these in `allowed-tools` and `requires_approval`. \
Claude Code tool names such as Read, Write, Edit, Glob, Grep and Bash do not exist in Nandi.\n\
\n\
## Proposing a change\n\
Before you show any skill.md or workflow, call `{CHECK_ARTIFACT}` with its full text and fix every error \
it reports (and warnings where you can). Then show the complete file once, in one fenced code block, \
with its path on the line above. Never show a file that failed the check.\n\
\n\
Then save it with `{SAVE_ARTIFACT}` in the same turn: the user approves the save on a card, so don't ask \
\"shall I save it?\" first. Never tell the user to copy a file or create a folder. Build what something \
needs in order: connectors, then agents, then workflows, then triggers. Secrets never go in a file or a \
tool call: say where the user enters them (Library › Browse › Tools, Settings › Models & runtimes).\n\
\n\
You can ask people on the user's Nandi network to do things with `delegate_to_agent`.\n\
{network}\n\
{open}{companion}\n\
Reply in Markdown.",
        network = facts.network_section,
        guide_index = guides::index_lines(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::tools::builtin::builtin_tools;

    fn facts() -> PromptFacts {
        PromptFacts {
            tool_ids: builtin_tools().into_iter().map(|t| t.name).collect(),
            default_model: "claude:claude-sonnet-5".into(),
            usable_models: vec!["claude:claude-sonnet-5".into()],
            network_section: String::new(),
            companion: None,
            open_artifact: None,
        }
    }

    #[test]
    fn an_open_file_is_what_this_means() {
        assert!(!studio_prompt(&facts()).contains("## Open in the editor"));
        let prompt = studio_prompt(&PromptFacts {
            open_artifact: Some(ArtifactRef::Agent("writer".into())),
            ..facts()
        });
        assert!(prompt.contains("the agent `writer` open"));
        assert!(prompt.contains(GET_AGENT));
    }

    #[test]
    fn the_companion_section_is_only_in_the_companions_conversation() {
        assert!(!studio_prompt(&facts()).contains("## In the companion"));

        let global = studio_prompt(&PromptFacts {
            companion: Some(CompanionFacts::default()),
            ..facts()
        });
        assert!(global.contains("## In the companion"));
        assert!(global.contains(REQUEST_LINK_PREFIX));
        assert!(!global.contains(GET_REQUEST), "no scope, no pinned Request");

        let scoped = studio_prompt(&PromptFacts {
            companion: Some(CompanionFacts {
                request_id: Some("task-7".into()),
            }),
            ..facts()
        });
        assert!(scoped.contains("about the Request `task-7`"));
        assert!(scoped.contains(GET_REQUEST));
    }

    #[test]
    fn the_prompt_matches_the_code() {
        let prompt = studio_prompt(&facts());
        // Every schema key, every provider prefix, every built-in tool
        for (key, _) in SKILL_FRONTMATTER_KEYS {
            assert!(prompt.contains(&format!("`{key}`")), "missing key {key}");
        }
        for provider in Provider::ALL {
            assert!(prompt.contains(&format!("`{}`", provider.prefix())));
        }
        for tool in builtin_tools() {
            assert!(
                prompt.contains(&format!("`{}`", tool.name)),
                "missing {}",
                tool.name
            );
        }
        assert!(prompt.contains("`model: claude:claude-sonnet-5`"));
        assert!(prompt.contains(LIST_RUNS) && prompt.contains(CHECK_ARTIFACT));
        assert!(prompt.contains(SAVE_ARTIFACT));
        assert!(
            !prompt.contains("apply it in Ask"),
            "nothing in Ask applies it"
        );
    }

    #[test]
    fn the_prompt_lists_every_guide_and_grows_only_by_the_index() {
        let prompt = studio_prompt(&facts());
        for guide in guides::all() {
            assert!(
                prompt.contains(&format!("`{}`", guide.id)),
                "missing {}",
                guide.id
            );
            assert!(
                !prompt.contains(&guide.body),
                "{}'s body is fetched, not inlined",
                guide.id
            );
        }
        assert!(prompt.contains(GET_GUIDE) && prompt.contains(START_REQUEST));
        let start = prompt.find("## How to do things").unwrap();
        let end = prompt.find("## Where things live").unwrap();
        let section = &prompt[start..end];
        // 13 guides at about 80 bytes each; a guide's `when` stays one short line
        assert!(
            section.len() < 1152,
            "the guide index is {} bytes",
            section.len()
        );
    }

    #[test]
    fn the_prompt_has_the_real_paths_only() {
        let prompt = studio_prompt(&facts());
        assert!(prompt.contains("~/.nandi/repo/.nandi/agents/<id>/skill.md"));
        // The old prompt's wrong location (review shot 27)
        assert!(!prompt.contains("~/.nandi/agents"));
        assert!(prompt.contains("do not exist in Nandi"));
    }

    #[test]
    fn without_a_key_the_prompt_says_where_to_add_one() {
        let prompt = studio_prompt(&PromptFacts {
            usable_models: vec![],
            ..facts()
        });
        assert!(prompt.contains("Settings › Models & runtimes"));
    }

    #[test]
    fn the_layout_matches_the_repo() {
        use crate::services::domain::workflow::WorkflowScope;
        let layout = crate::services::repos::RepoLayout::new("/h/.nandi/repo", "/h/.nandi/trash");
        let path =
            |p: std::path::PathBuf| p.to_string_lossy().replace('\\', "/").replace("/h/", "~/");
        let real = [
            path(layout.skill_path("<id>")),
            path(layout.instructions_path("<id>")),
            path(layout.workflow_path("<id>", &WorkflowScope::Global)),
            path(layout.trigger_path("<id>")),
            path(layout.settings_path()),
        ];
        let documented: Vec<&str> = REPO_LAYOUT.iter().map(|(p, _)| *p).collect();
        assert_eq!(real.to_vec(), documented);
    }
}
