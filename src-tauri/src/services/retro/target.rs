//! Which file a proposal changes, and whether the change keeps to the
//! retro's limits (`domain::retro`). Checked when Nandi proposes it and
//! again when the person applies it.

use std::path::PathBuf;

use crate::services::domain::agent::Agent;
use crate::services::domain::playbook::{self, Playbook};
use crate::services::domain::retro::{check_agent, check_trigger, check_workflow};
use crate::services::domain::{AgentId, EntityKind, Trigger, Workflow, WorkflowId};
use crate::services::repo_store::RepoStore;
use crate::services::repos::formats;
use crate::services::repos::layout::{AGENT_FILE, PLAYBOOKS_DIR};

/// What a proposal changes
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// The agent's `agent.md`
    Agent,
    /// One of the agent's playbooks (`skills/<name>.md`), new or not
    Playbook(String),
    /// A workflow, by ID
    Workflow(String),
    /// A trigger, by ID
    Trigger(String),
}

impl Target {
    /// `agent`, `playbook:<name>`, `workflow:<id>` or `trigger:<id>`
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let named = |prefix: &str| {
            text.strip_prefix(prefix)
                .map(str::trim)
                .filter(|n| !n.is_empty())
        };
        if text == "agent" {
            Ok(Self::Agent)
        } else if let Some(name) = named("playbook:") {
            Ok(Self::Playbook(name.to_string()))
        } else if let Some(id) = named("workflow:") {
            Ok(Self::Workflow(id.to_string()))
        } else if let Some(id) = named("trigger:") {
            Ok(Self::Trigger(id.to_string()))
        } else {
            Err(format!(
                "'{text}' isn't something the retro may change: use `agent`, `playbook:<name>`, \
`workflow:<id>` or `trigger:<id>`"
            ))
        }
    }

    pub fn as_string(&self) -> String {
        match self {
            Self::Agent => "agent".to_string(),
            Self::Playbook(name) => format!("playbook:{name}"),
            Self::Workflow(id) => format!("workflow:{id}"),
            Self::Trigger(id) => format!("trigger:{id}"),
        }
    }
}

/// A target found in the gold copy
#[derive(Debug, Clone)]
pub struct Resolved {
    pub path: PathBuf,
    /// Relative to the root, `/` separators
    pub relative: String,
    /// The file now; `None` when it doesn't exist (a new playbook)
    pub current: Option<String>,
    /// What to reload once it's written
    pub kind: EntityKind,
    pub entity_id: String,
}

/// The refusal for an agent installed from a pack
pub const PACK_AGENT: &str = "came from a signed pack, so its files are never edited. Propose \
nothing for it; suggest in your summary that the person make a local copy of it (Fork, in the \
Library) if they want it to learn.";

/// Finds `target` of agent `agent_id`: a local agent's file, never one
/// from a pack
pub fn resolve(store: &RepoStore, agent_id: &str, target: &Target) -> Result<Resolved, String> {
    let repos = store.repos();
    let layout = store.layout();
    let place = repos
        .agents
        .locate(agent_id)
        .ok_or_else(|| format!("There is no agent '{agent_id}'"))?;
    if place.origin.is_vendored() {
        return Err(format!("Agent '{agent_id}' {PACK_AGENT}"));
    }
    let (path, kind, entity_id, must_exist) = match target {
        Target::Agent => (
            place.dir.join(AGENT_FILE),
            EntityKind::Agent,
            agent_id.to_string(),
            true,
        ),
        Target::Playbook(name) => {
            if !playbook::is_valid_name(name) {
                return Err(format!("'{name}' isn't a playbook name (kebab-case)"));
            }
            (
                place.dir.join(PLAYBOOKS_DIR).join(format!("{name}.md")),
                EntityKind::Agent,
                agent_id.to_string(),
                false,
            )
        }
        Target::Workflow(id) => {
            let workflow_id = WorkflowId::parse(id).map_err(|e| e.to_string())?;
            let (path, _) = repos
                .workflows
                .locate(&workflow_id)
                .ok_or_else(|| format!("There is no workflow '{id}'"))?;
            (path, EntityKind::Workflow, id.clone(), true)
        }
        Target::Trigger(id) => {
            let path = layout.trigger_path(id);
            (path, EntityKind::Trigger, id.clone(), true)
        }
    };
    if layout.is_vendored(&path) {
        return Err(format!("{} {PACK_AGENT}", path.display()));
    }
    let current = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("Couldn't read {}: {e}", path.display())),
    };
    if must_exist && current.is_none() {
        return Err(format!(
            "{} doesn't exist: the retro changes what is there, it doesn't add it",
            target.as_string()
        ));
    }
    let relative = layout
        .relative(&path)
        .ok_or_else(|| format!("{} is outside the gold copy", path.display()))?;
    Ok(Resolved {
        path,
        relative,
        current,
        kind,
        entity_id,
    })
}

fn agent(id: &str, text: &str) -> Result<Agent, String> {
    let id = AgentId::parse(id).map_err(|e| e.to_string())?;
    let text = formats::AGENT.read(text).map_err(|e| e.to_string())?;
    Agent::from_files(id, &text, None).map_err(|e| e.to_string())
}

fn workflow(text: &str) -> Result<Workflow, String> {
    let text = formats::WORKFLOW.read(text).map_err(|e| e.to_string())?;
    Workflow::from_yaml(&text).map_err(|e| e.to_string())
}

fn trigger(text: &str) -> Result<Trigger, String> {
    let text = formats::TRIGGER.read(text).map_err(|e| e.to_string())?;
    Trigger::from_yaml(&text).map_err(|e| e.to_string())
}

/// Whether `after` may replace `before` as `target` of agent `agent_id`:
/// it parses, and it widens nothing (`domain::retro`). The reason when not.
pub fn check(
    agent_id: &str,
    target: &Target,
    before: Option<&str>,
    after: &str,
) -> Result<(), String> {
    let refused = |why: String| format!("Refused: {why}. A retro change may never widen access.");
    match target {
        Target::Agent => {
            let before = agent(agent_id, before.unwrap_or_default())?;
            let after = agent(agent_id, after).map_err(|e| format!("The new agent.md: {e}"))?;
            check_agent(&before, &after).map_err(|w| refused(w.to_string()))
        }
        Target::Playbook(name) => Playbook::from_md(name, after)
            .map(|_| ())
            .map_err(|e| format!("The playbook: {e}")),
        Target::Workflow(id) => {
            let before = workflow(before.unwrap_or_default())?;
            let after = workflow(after).map_err(|e| format!("The new workflow: {e}"))?;
            if after.id().as_str() != id {
                return Err(refused("it changes another workflow".to_string()));
            }
            check_workflow(&before, &after).map_err(|w| refused(w.to_string()))
        }
        Target::Trigger(id) => {
            let before = trigger(before.unwrap_or_default())?;
            let after = trigger(after).map_err(|e| format!("The new trigger: {e}"))?;
            if after.id().as_str() != id {
                return Err(refused("it changes another trigger".to_string()));
            }
            check_trigger(&before, &after).map_err(|w| refused(w.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_round_trip_and_others_are_refused() {
        for text in [
            "agent",
            "playbook:pay-a-bill",
            "workflow:weekly",
            "trigger:digest",
        ] {
            assert_eq!(Target::parse(text).unwrap().as_string(), text);
        }
        for text in [
            "",
            "instructions",
            "playbook:",
            "policy:approvals",
            "settings",
        ] {
            assert!(Target::parse(text).is_err(), "{text}");
        }
    }
}
