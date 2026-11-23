//! The weekly retro's limits (task "Weekly retro"): what a change Nandi
//! proposes from the person's corrections may do, and what it may never do.
//! Checked in code when a proposal is made and again when the person
//! applies it, never only in the prompt. Pure.
//!
//! | ✅ A change may                                    | ❌ A change may never                                  |
//! | -------------------------------------------------- | ------------------------------------------------------ |
//! | Edit a `skill.md`'s text, a playbook, a workflow's  | Add a tool, a site, a secret, a connector or a pilot   |
//! | steps' wording or a trigger's schedule and input   | Remove an approval, or let a pilot act without asking  |
//! | Narrow the tools, sites, connectors or pilots      | Touch the sandbox, the runtime or the AWS grants       |
//! |                                                    | Add a workflow step or change who does one             |
//! |                                                    | Change what starts a trigger (other than its schedule) |
//!
//! Signed pack agents are never edited: that is checked where the agent's
//! origin is known (`services::retro`).

use std::fmt;

use serde_yaml::Value as Yaml;

use super::agent::Agent;
use super::capability::{CapabilitySet, HostRule};
use super::trigger::{Trigger, TriggerSource};
use super::workflow::{Step, Workflow};

/// Why a proposed change is refused: it would give an agent more than it
/// has now
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Widening {
    /// A tool the agent may not call now
    AddsTool(String),
    /// A tool that asks first now would run without asking
    DropsApproval(String),
    /// A site the agent can't reach now, or more methods on one
    AddsSite(String),
    /// A secret the agent has no access to now
    AddsSecret(String),
    /// A connector, or more of one (a folder, write access)
    AddsConnector(String),
    /// A pilot, or a pilot action that runs without asking
    AddsPilot(String),
    /// The container, the runtime or its settings
    TouchesSandbox,
    /// AWS access
    TouchesAws,
    /// Others may send it work, or it serves more
    WidensDelegation,
    /// A frontmatter key this build doesn't know: it may be a grant
    TouchesUnknown(String),
    /// A workflow step that wasn't there, or a step done by someone else
    ChangesStep(String),
    /// A workflow step that asks a person, or that a person does, would go
    DropsStepApproval(String),
    /// A workflow that provides more to others
    AddsProvides(String),
    /// What starts the trigger or what it runs
    ChangesTrigger,
    /// A different agent, workflow or trigger than the one proposed
    ChangesId,
}

impl fmt::Display for Widening {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AddsTool(tool) => write!(f, "it adds the tool '{tool}'"),
            Self::DropsApproval(tool) => {
                write!(f, "it lets '{tool}' run without asking the person first")
            }
            Self::AddsSite(site) => write!(f, "it adds the site '{site}'"),
            Self::AddsSecret(name) => write!(f, "it adds the secret '{name}'"),
            Self::AddsConnector(id) => write!(f, "it adds or widens the connector '{id}'"),
            Self::AddsPilot(id) => write!(f, "it adds or widens the pilot '{id}'"),
            Self::TouchesSandbox => write!(f, "it changes the sandbox or the runtime"),
            Self::TouchesAws => write!(f, "it changes AWS access"),
            Self::WidensDelegation => write!(f, "it lets others send this agent more work"),
            Self::TouchesUnknown(key) => write!(f, "it changes the setting '{key}'"),
            Self::ChangesStep(step) => {
                write!(f, "it adds the step '{step}' or changes who does it")
            }
            Self::DropsStepApproval(step) => {
                write!(
                    f,
                    "it removes the person's approval or hand-off at step '{step}'"
                )
            }
            Self::AddsProvides(id) => write!(f, "it makes the workflow provide '{id}'"),
            Self::ChangesTrigger => write!(f, "it changes what starts the trigger or what it runs"),
            Self::ChangesId => write!(f, "it changes a different definition"),
        }
    }
}

/// Frontmatter keys an agent's text may change freely
const FREE_EXTRA_KEYS: &[&str] = &["tags", "connectors"];

/// `after` may replace `before` (the same agent's `skill.md`): only its
/// text, name and description change, or it narrows what it may do
pub fn check_agent(before: &Agent, after: &Agent) -> Result<(), Widening> {
    if before.id() != after.id() {
        return Err(Widening::ChangesId);
    }
    if before.container() != after.container()
        || before.runtime() != after.runtime()
        || before.runtime_config() != after.runtime_config()
    {
        return Err(Widening::TouchesSandbox);
    }
    check_capabilities(before.capabilities(), after.capabilities())?;
    check_extra(before, after)
}

fn check_capabilities(before: &CapabilitySet, after: &CapabilitySet) -> Result<(), Widening> {
    let (old, new) = (before.tools(), after.tools());
    // `None` is every runtime tool: anything is narrower, nothing is wider
    if let Some(old_allowed) = old.allowed() {
        match new.allowed() {
            None => return Err(Widening::AddsTool("every tool".to_string())),
            Some(new_allowed) => {
                if let Some(tool) = new_allowed.iter().find(|t| !old_allowed.contains(t)) {
                    return Err(Widening::AddsTool(tool.clone()));
                }
            }
        }
    }
    let still_asks = new.approval_list();
    if let Some(tool) = old
        .approval_list()
        .into_iter()
        .find(|tool| new.allows(tool) && !still_asks.contains(tool))
    {
        return Err(Widening::DropsApproval(tool));
    }
    for rule in &after.network().allow {
        if !before.network().allow.iter().any(|old| narrows(rule, old)) {
            return Err(Widening::AddsSite(rule.host.clone()));
        }
    }
    for pilot in after.pilots() {
        if !before.pilots().contains(pilot) {
            return Err(Widening::AddsPilot(pilot.as_str().to_string()));
        }
        let (old, new) = (
            before
                .pilot_options(pilot.as_str())
                .cloned()
                .unwrap_or_default(),
            after
                .pilot_options(pilot.as_str())
                .cloned()
                .unwrap_or_default(),
        );
        if new.allow.iter().any(|kind| !old.allow.contains(kind)) {
            return Err(Widening::AddsPilot(pilot.as_str().to_string()));
        }
        if let Some((name, _)) = new
            .secrets
            .iter()
            .find(|(name, entry)| old.secrets.get(*name) != Some(*entry))
        {
            return Err(Widening::AddsSecret(name.clone()));
        }
    }
    if after.aws().is_some() && before.aws() != after.aws() {
        return Err(Widening::TouchesAws);
    }
    let serves_more = after
        .serves()
        .iter()
        .any(|s| !before.serves().iter().any(|old| old.name == s.name));
    if serves_more
        || (after.accepts_delegation() && !before.accepts_delegation())
        || after.delegation_limit() > before.delegation_limit()
    {
        return Err(Widening::WidensDelegation);
    }
    Ok(())
}

/// Whether `rule` reaches no more than `old`: the same host, and no method
/// `old` doesn't allow (no methods is every method)
fn narrows(rule: &HostRule, old: &HostRule) -> bool {
    rule.host == old.host
        && (old.methods.is_empty()
            || (!rule.methods.is_empty() && rule.methods.iter().all(|m| old.methods.contains(m))))
}

fn check_extra(before: &Agent, after: &Agent) -> Result<(), Widening> {
    let (old, new) = (before.extra(), after.extra());
    for key in old.keys().chain(new.keys()) {
        if !FREE_EXTRA_KEYS.contains(&key.as_str()) && old.get(key) != new.get(key) {
            return Err(Widening::TouchesUnknown(key.clone()));
        }
    }
    let entries = |value: Option<&Yaml>| -> Vec<Yaml> {
        match value {
            Some(Yaml::Sequence(items)) => items.clone(),
            _ => Vec::new(),
        }
    };
    let old_entries = entries(old.get("connectors"));
    for entry in entries(new.get("connectors")) {
        let id = connector_id(&entry);
        let narrower = old_entries
            .iter()
            .any(|old| connector_id(old) == id && connector_narrows(&entry, old));
        if !narrower {
            return Err(Widening::AddsConnector(id));
        }
    }
    Ok(())
}

/// A `connectors:` entry's ID: `filesystem` or `{id: filesystem, ...}`
fn connector_id(entry: &Yaml) -> String {
    match entry {
        Yaml::String(id) => id.clone(),
        Yaml::Mapping(map) => map
            .get("id")
            .and_then(Yaml::as_str)
            .unwrap_or_default()
            .to_string(),
        _ => String::new(),
    }
}

/// Whether `entry` grants no more than `old`: the same entry, or the same
/// one with fewer folders or read access where it had write
fn connector_narrows(entry: &Yaml, old: &Yaml) -> bool {
    if entry == old {
        return true;
    }
    let (Yaml::Mapping(new), Yaml::Mapping(old)) = (entry, old) else {
        return false;
    };
    let others_same = new
        .iter()
        .filter(|(key, _)| key.as_str() != Some("folders"))
        .all(|(key, value)| old.get(key) == Some(value))
        && old
            .iter()
            .filter(|(key, _)| key.as_str() != Some("folders"))
            .all(|(key, _)| new.contains_key(key));
    if !others_same {
        return false;
    }
    let folders = |map: &serde_yaml::Mapping| -> Vec<Yaml> {
        match map.get("folders") {
            Some(Yaml::Sequence(items)) => items.clone(),
            _ => Vec::new(),
        }
    };
    let old_folders = folders(old);
    folders(new).iter().all(|folder| {
        old_folders.iter().any(|was| {
            folder.get("id") == was.get("id")
                && (folder.get("access") == was.get("access")
                    || folder.get("access").and_then(Yaml::as_str) == Some("read"))
        })
    })
}

/// `after` may replace `before` (the same workflow): its steps' wording,
/// deadlines and failure handling change, steps go, or one asks first. A
/// step that asks or hands off to the person stays.
pub fn check_workflow(before: &Workflow, after: &Workflow) -> Result<(), Widening> {
    if before.id() != after.id() || before.scope() != after.scope() {
        return Err(Widening::ChangesId);
    }
    if let Some(id) = after
        .provides()
        .iter()
        .find(|id| !before.provides().contains(id))
    {
        return Err(Widening::AddsProvides(id.clone()));
    }
    let find = |steps: &[Step], id: &str| steps.iter().find(|s| s.id.as_str() == id).cloned();
    for step in after.steps() {
        let Some(old) = find(before.steps(), step.id.as_str()) else {
            return Err(Widening::ChangesStep(step.id.as_str().to_string()));
        };
        if step.step_type != old.step_type || step.agent != old.agent || step.ask != old.ask {
            return Err(Widening::ChangesStep(step.id.as_str().to_string()));
        }
    }
    for old in before.steps() {
        let gate = old.requires_approval || old.is_manual() || old.is_ask();
        if !gate {
            continue;
        }
        let kept = find(after.steps(), old.id.as_str())
            .is_some_and(|step| !old.requires_approval || step.requires_approval);
        if !kept {
            return Err(Widening::DropsStepApproval(old.id.as_str().to_string()));
        }
    }
    Ok(())
}

/// `after` may replace `before` (the same trigger): its name, schedule,
/// input, priority and rate change, or it's switched on or off. What starts
/// it (other than a schedule's time) and what it runs stay.
pub fn check_trigger(before: &Trigger, after: &Trigger) -> Result<(), Widening> {
    if before.id() != after.id() {
        return Err(Widening::ChangesId);
    }
    if before.target() != after.target() {
        return Err(Widening::ChangesTrigger);
    }
    match (before.source(), after.source()) {
        (TriggerSource::Cron(_), TriggerSource::Cron(_)) => Ok(()),
        (old, new) if old == new => Ok(()),
        _ => Err(Widening::ChangesTrigger),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::AgentId;

    const BASE: &str = "---
name: Helper
description: Helps
allowed-tools: [read_file, write_file, shell]
requires_approval: [write_file, shell]
network:
  allow:
    - host: api.example.com
    - host: shop.example
      methods: [GET, POST]
connectors:
  - id: filesystem
    folders: [{ id: my-notes, access: write }, { id: work, access: read }]
pilots:
  - id: browser
    allow: [interact]
---

Be brief.
";

    fn agent(content: &str) -> Agent {
        Agent::from_skill_md(AgentId::parse("helper").unwrap(), content).unwrap()
    }

    fn changed(from: &str, to: &str) -> Result<(), Widening> {
        check_agent(&agent(BASE), &agent(&BASE.replacen(from, to, 1)))
    }

    #[test]
    fn the_text_and_narrowing_are_allowed() {
        assert_eq!(
            changed("Be brief.", "Be brief. Lead with the answer."),
            Ok(())
        );
        assert_eq!(
            changed("description: Helps", "description: Helps you"),
            Ok(())
        );
        // Fewer tools, sites, methods, folders, write → read, no pilot
        assert_eq!(
            changed("read_file, write_file, shell", "read_file, write_file"),
            Ok(())
        );
        assert_eq!(changed("    - host: api.example.com\n", ""), Ok(()));
        assert_eq!(changed("methods: [GET, POST]", "methods: [GET]"), Ok(()));
        assert_eq!(changed(", { id: work, access: read }", ""), Ok(()));
        assert_eq!(
            changed("my-notes, access: write", "my-notes, access: read"),
            Ok(())
        );
        assert_eq!(changed("    allow: [interact]\n", ""), Ok(()));
        assert_eq!(
            changed("[write_file, shell]", "[write_file, shell, read_file]"),
            Ok(())
        );
    }

    #[test]
    fn a_tool_a_site_or_a_secret_is_refused() {
        assert_eq!(
            changed(
                "read_file, write_file, shell",
                "read_file, write_file, shell, http_fetch"
            ),
            Err(Widening::AddsTool("http_fetch".into()))
        );
        assert_eq!(
            changed("allowed-tools: [read_file, write_file, shell]\n", ""),
            Err(Widening::AddsTool("every tool".into()))
        );
        assert_eq!(
            changed("- host: api.example.com", "- host: evil.example"),
            Err(Widening::AddsSite("evil.example".into()))
        );
        assert_eq!(
            changed("methods: [GET, POST]", "methods: [GET, POST, DELETE]"),
            Err(Widening::AddsSite("shop.example".into()))
        );
        assert_eq!(
            changed("\n      methods: [GET, POST]", ""),
            Err(Widening::AddsSite("shop.example".into()))
        );
        assert_eq!(
            changed(
                "allow: [interact]",
                "allow: [interact]\n    secrets: {SITE_PASSWORD: nandi.pilot.helper.site}"
            ),
            Err(Widening::AddsSecret("SITE_PASSWORD".into()))
        );
    }

    #[test]
    fn an_approval_a_connector_a_pilot_or_the_sandbox_is_refused() {
        assert_eq!(
            changed("[write_file, shell]", "[write_file]"),
            Err(Widening::DropsApproval("shell".into()))
        );
        assert_eq!(
            changed("{ id: work, access: read }", "{ id: work, access: write }"),
            Err(Widening::AddsConnector("filesystem".into()))
        );
        assert_eq!(
            changed(
                "{ id: work, access: read }",
                "{ id: work, access: read }, { id: home, access: read }"
            ),
            Err(Widening::AddsConnector("filesystem".into()))
        );
        assert_eq!(
            changed("connectors:\n", "connectors:\n  - github\n"),
            Err(Widening::AddsConnector("github".into()))
        );
        assert_eq!(
            changed("  - id: browser\n", "  - id: os-control\n  - id: browser\n"),
            Err(Widening::AddsPilot("os-control".into()))
        );
        assert_eq!(
            changed("allow: [interact]", "allow: [interact, submit]"),
            Err(Widening::AddsPilot("browser".into()))
        );
        assert_eq!(
            changed(
                "description: Helps\n",
                "description: Helps\nmemory_mb: 4096\n"
            ),
            Err(Widening::TouchesSandbox)
        );
        assert_eq!(
            changed(
                "description: Helps\n",
                "description: Helps\naccepts_delegation: true\n"
            ),
            Err(Widening::WidensDelegation)
        );
    }

    fn workflow(steps: &str) -> Workflow {
        Workflow::from_yaml(&format!("id: weekly\nname: Weekly\nsteps:\n{steps}")).unwrap()
    }

    const STEPS: &str = "  - id: draft
    agent: helper
    input: Draft it
  - id: send
    agent: helper
    input: Send it
    requires_approval: true
";

    #[test]
    fn a_workflow_changes_wording_but_keeps_its_gates_and_performers() {
        let before = workflow(STEPS);
        let reworded = workflow(&STEPS.replace("Draft it", "Draft it in three lines"));
        assert_eq!(check_workflow(&before, &reworded), Ok(()));
        let no_gate = workflow(&STEPS.replace("    requires_approval: true\n", ""));
        assert_eq!(
            check_workflow(&before, &no_gate),
            Err(Widening::DropsStepApproval("send".into()))
        );
        let other_agent = workflow(&STEPS.replacen("agent: helper", "agent: shopper", 1));
        assert_eq!(
            check_workflow(&before, &other_agent),
            Err(Widening::ChangesStep("draft".into()))
        );
        let added = workflow(&format!(
            "{STEPS}  - id: post\n    agent: helper\n    input: Post\n"
        ));
        assert_eq!(
            check_workflow(&before, &added),
            Err(Widening::ChangesStep("post".into()))
        );
    }

    fn trigger(source: &str) -> Trigger {
        Trigger::from_yaml(&format!(
            "id: digest\nname: Digest\nsource:\n{source}target:\n  type: agent\n  id: nandi\ninput_template: Brief me\n"
        ))
        .unwrap()
    }

    #[test]
    fn a_trigger_changes_its_time_and_input_but_not_what_starts_it() {
        let cron = "  type: cron\n  config:\n    schedule: '0 8 * * *'\n";
        let before = trigger(cron);
        assert_eq!(
            check_trigger(&before, &trigger(&cron.replace("0 8", "30 7"))),
            Ok(())
        );
        let webhook = trigger("  type: webhook\n  config: {}\n");
        assert_eq!(
            check_trigger(&before, &webhook),
            Err(Widening::ChangesTrigger)
        );
    }
}
