//! The workflow entity: `.nandi/workflows/<id>.yaml` (global) or
//! `.nandi/agents/<agent>/workflows/<id>.yaml` (agent-scoped).
//!
//! ```yaml
//! id: daily-report
//! name: Daily Report
//! steps:
//!   - id: research
//!     agent: research-agent
//!     input: "Find today's AI news"
//!   - id: summarize
//!     agent: writer-agent
//!     input: "Summarize: {{research.output}}"
//!     critical: true
//!     on_failure: continue      # default: fail the workflow
//!   - id: publish
//!     agent: publisher-agent
//!     input: "Send to #ai-news: {{summarize.output}}"
//!     requires_approval: true
//! provides: [daily-report.ai]   # optional: capability IDs
//! ```
//!
//! A workflow that `provides:` a capability can run delegated tasks for
//! it; a step's `{{input}}` (or `{{input.<field>}}`) is the task's input.

use std::collections::HashSet;
use std::sync::LazyLock;
use std::time::{SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::{check_len, AgentId, DomainError, DomainResult, StepId, WorkflowId};

const MAX_INPUT_CHARS: usize = 100_000;

/// `{{ step.output }}`
static OUTPUT_REF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\{\{\s*([A-Za-z0-9_-]+)\.output\s*\}\}").unwrap());

/// What a failed step does to its workflow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailurePolicy {
    /// The workflow fails (the default)
    #[default]
    Fail,
    /// Later steps run; this step's output is empty
    Continue,
}

impl FailurePolicy {
    fn is_default(&self) -> bool {
        *self == Self::Fail
    }
}

/// What a step does
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    /// Sends `input` to `agent` (the default)
    #[default]
    Agent,
    /// Waits for a person to follow `instructions` and complete it; the
    /// run is suspended (possibly for days) until then
    Manual,
}

impl StepType {
    fn is_default(&self) -> bool {
        *self == Self::Agent
    }
}

/// A step that asks a colleague: one
/// request to whoever owns `find`, to `person`'s Nandi, or to any provider
/// of `capability`. The run waits for the answer (days if it must), which
/// is `{{step.output}}` for later steps. One person or capability per step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AskStep {
    /// A topic: the person whose responsibilities match best, asked
    /// through their Nandi. The owner approves who it is before it's sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub find: Option<String>,
    /// An instance ID: that person's Nandi
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<String>,
    /// Any provider of this capability (e.g. `review-pr`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capability: Option<String>,
    /// What to ask; `{{step.output}}` is an earlier step's output
    pub input: String,
    /// `YYYY-MM-DD`, an RFC 3339 time, or `N days` (else the request's
    /// default: 2 days for a question, 1 for anything else)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: StepId,
    #[serde(
        default,
        alias = "stepType",
        skip_serializing_if = "StepType::is_default"
    )]
    pub step_type: StepType,
    /// The agent an `agent` step messages; manual steps have none
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentId>,
    /// Message sent to the agent; `{{step.output}}` is an earlier step's reply
    #[serde(rename = "input", default, skip_serializing_if = "String::is_empty")]
    pub input_template: String,
    /// What the person should do (manual steps); may use `{{step.output}}`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
    /// Manual steps: due this many hours after the run reaches them
    #[serde(
        default,
        alias = "deadlineHours",
        skip_serializing_if = "Option::is_none"
    )]
    pub deadline_hours: Option<u32>,
    /// Pause for the user's approval before sending this step's message
    #[serde(default, alias = "requiresApproval", skip_serializing_if = "is_false")]
    pub requires_approval: bool,
    /// Later steps depend on it: queue ahead of normal workflow work
    #[serde(default, skip_serializing_if = "is_false")]
    pub critical: bool,
    #[serde(
        default,
        alias = "onFailure",
        skip_serializing_if = "FailurePolicy::is_default"
    )]
    pub on_failure: FailurePolicy,
    /// Asks a colleague instead of messaging a local agent
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<AskStep>,
}

fn is_false(value: &bool) -> bool {
    !value
}

impl Step {
    /// The step IDs this step's input (or instructions) refers to.
    pub fn references(&self) -> impl Iterator<Item = &str> {
        let instructions = self.instructions.as_deref().unwrap_or_default();
        let asked = self.ask.as_ref().map_or("", |ask| ask.input.as_str());
        OUTPUT_REF
            .captures_iter(&self.input_template)
            .chain(OUTPUT_REF.captures_iter(instructions))
            .chain(OUTPUT_REF.captures_iter(asked))
            .filter_map(|c| c.get(1).map(|m| m.as_str()))
    }

    /// Asks a colleague (an `ask:` block)
    pub fn is_ask(&self) -> bool {
        self.ask.is_some()
    }

    pub fn is_manual(&self) -> bool {
        self.step_type == StepType::Manual
    }

    /// Who does the step, for summaries: the agent ID, `ask`, or `manual`
    pub fn performer(&self) -> &str {
        if self.is_ask() {
            return "ask";
        }
        self.agent.as_ref().map_or("manual", AgentId::as_str)
    }
}

/// Where a workflow file lives.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum WorkflowScope {
    #[default]
    Global,
    Agent(AgentId),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Workflow {
    id: WorkflowId,
    name: String,
    /// What it does, in a sentence or two (Automations › Overview)
    description: Option<String>,
    steps: Vec<Step>,
    /// Capability IDs this workflow provides
    provides: Vec<String>,
    scope: WorkflowScope,
    created_at: SystemTime,
    updated_at: SystemTime,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorkflowFile {
    id: WorkflowId,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    steps: Vec<Step>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    provides: Vec<String>,
}

/// A capability ID as `provides:` may name it (`capability::definition`'s
/// pattern: lowercase letters, digits, dots, and dashes)
fn valid_capability_id(id: &str) -> bool {
    let mut chars = id.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
}

impl Workflow {
    pub fn new(id: WorkflowId, name: impl Into<String>, steps: Vec<Step>) -> DomainResult<Self> {
        let now = SystemTime::now();
        let workflow = Self {
            id,
            name: name.into().trim().to_string(),
            description: None,
            steps,
            provides: Vec::new(),
            scope: WorkflowScope::Global,
            created_at: now,
            updated_at: now,
        };
        workflow.check()?;
        Ok(workflow)
    }

    fn label(&self) -> String {
        format!("Workflow '{}'", self.id)
    }

    /// Structural invariants: a name, at least one step, unique step IDs,
    /// non-empty inputs, and references only to earlier steps.
    fn check(&self) -> DomainResult<()> {
        let label = self.label();
        if self.name.is_empty() {
            return Err(DomainError::invalid(label, "name", "is empty"));
        }
        if self.steps.is_empty() {
            return Err(DomainError::Rule(format!("{label} has no steps")));
        }
        let all: HashSet<&str> = self.steps.iter().map(|s| s.id.as_str()).collect();
        let mut earlier = HashSet::new();
        for step in &self.steps {
            if let Some(ask) = &step.ask {
                let named = [&ask.find, &ask.person, &ask.capability]
                    .iter()
                    .filter(|value| value.as_deref().is_some_and(|v| !v.trim().is_empty()))
                    .count();
                let problem = if named != 1 {
                    Some("must name exactly one of find, person or capability")
                } else if step.step_type == StepType::Manual || step.agent.is_some() {
                    Some("asks a colleague, so it can't have an agent or be manual")
                } else if ask.input.trim().is_empty() {
                    Some("has nothing to ask (ask.input is empty)")
                } else {
                    None
                };
                if let Some(problem) = problem {
                    return Err(DomainError::invalid(
                        &label,
                        "steps",
                        format!("ask step '{}' {problem}", step.id),
                    ));
                }
                check_len(&label, "step input", &ask.input, MAX_INPUT_CHARS)?;
            } else {
                match step.step_type {
                    StepType::Agent => {
                        if step.agent.is_none() {
                            return Err(DomainError::invalid(
                                &label,
                                "steps",
                                format!("'{}' has no agent", step.id),
                            ));
                        }
                        if step.input_template.trim().is_empty() {
                            return Err(DomainError::invalid(
                                &label,
                                "steps",
                                format!("'{}' has no input", step.id),
                            ));
                        }
                    }
                    StepType::Manual => {
                        let instructions = step.instructions.as_deref().unwrap_or_default();
                        if instructions.trim().is_empty() {
                            return Err(DomainError::invalid(
                                &label,
                                "steps",
                                format!("manual step '{}' has no instructions", step.id),
                            ));
                        }
                        check_len(&label, "step instructions", instructions, MAX_INPUT_CHARS)?;
                    }
                }
            }
            check_len(&label, "step input", &step.input_template, MAX_INPUT_CHARS)?;
            for reference in step.references() {
                if earlier.contains(reference) {
                    continue;
                }
                if all.contains(reference) {
                    // Itself or a later step: the graph would loop
                    return Err(DomainError::Cycle {
                        workflow: self.id.to_string(),
                        step: step.id.to_string(),
                        reference: reference.to_string(),
                    });
                }
                return Err(DomainError::DanglingReference {
                    from: format!("Workflow '{}' step '{}'", self.id, step.id),
                    to: format!("step '{reference}'"),
                });
            }
            if !earlier.insert(step.id.as_str()) {
                return Err(DomainError::Rule(format!(
                    "Duplicate step id '{}' in {label}",
                    step.id
                )));
            }
        }
        let mut provided = HashSet::new();
        for capability in &self.provides {
            if !valid_capability_id(capability) {
                return Err(DomainError::invalid(
                    &label,
                    "provides",
                    format!("has an invalid capability id '{capability}'"),
                ));
            }
            if !provided.insert(capability.as_str()) {
                return Err(DomainError::invalid(
                    &label,
                    "provides",
                    format!("lists '{capability}' twice"),
                ));
            }
        }
        Ok(())
    }

    /// The structural checks plus: every step's agent exists.
    pub fn validate_step_graph(&self, agent_exists: impl Fn(&AgentId) -> bool) -> DomainResult<()> {
        self.check()?;
        let missing = self.steps.iter().find_map(|s| {
            s.agent
                .as_ref()
                .filter(|a| !agent_exists(a))
                .map(|a| (s, a))
        });
        match missing {
            Some((step, agent)) => Err(DomainError::DanglingReference {
                from: format!("Workflow '{}' step '{}'", self.id, step.id),
                to: format!("agent '{agent}'"),
            }),
            None => Ok(()),
        }
    }

    pub fn from_yaml(content: &str) -> DomainResult<Self> {
        let file: WorkflowFile =
            serde_yaml::from_str(content).map_err(|e| DomainError::parse("workflow YAML", e))?;
        let workflow = Self {
            id: file.id,
            name: file.name.trim().to_string(),
            description: file
                .description
                .map(|d| d.trim().to_string())
                .filter(|d| !d.is_empty()),
            steps: file.steps,
            provides: file
                .provides
                .into_iter()
                .map(|c| c.trim().to_string())
                .collect(),
            scope: WorkflowScope::Global,
            created_at: UNIX_EPOCH,
            updated_at: UNIX_EPOCH,
        };
        workflow.check()?;
        Ok(workflow)
    }

    pub fn to_yaml(&self) -> String {
        let file = WorkflowFile {
            id: self.id.clone(),
            name: self.name.clone(),
            description: self.description.clone(),
            steps: self.steps.clone(),
            provides: self.provides.clone(),
        };
        serde_yaml::to_string(&file).unwrap_or_default()
    }

    /// Points steps that use agent `from` at agent `to` (after a fork), and
    /// moves an agent-scoped workflow of `from` to `to`. Returns whether
    /// anything changed.
    pub fn rename_agent(&mut self, from: &AgentId, to: &AgentId) -> bool {
        let mut changed = false;
        for step in &mut self.steps {
            if step.agent.as_ref() == Some(from) {
                step.agent = Some(to.clone());
                changed = true;
            }
        }
        if self.scope == WorkflowScope::Agent(from.clone()) {
            self.scope = WorkflowScope::Agent(to.clone());
            changed = true;
        }
        if changed {
            self.updated_at = SystemTime::now();
        }
        changed
    }

    pub fn in_scope(mut self, scope: WorkflowScope) -> Self {
        self.scope = scope;
        self
    }

    pub fn with_timestamps(mut self, created_at: SystemTime, updated_at: SystemTime) -> Self {
        self.created_at = created_at;
        self.updated_at = updated_at;
        self
    }

    /// Same ID, name, steps, and `provides:` (scope and timestamps aside).
    pub fn same_definition(&self, other: &Self) -> bool {
        self.id == other.id
            && self.name == other.name
            && self.steps == other.steps
            && self.provides == other.provides
    }

    /// Capability IDs this workflow provides (`provides:`)
    pub fn provides(&self) -> &[String] {
        &self.provides
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn with_description(mut self, description: Option<String>) -> Self {
        self.description = description
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty());
        self
    }

    /// The agents this workflow sends work to, in step order (manual steps
    /// have none).
    pub fn agents(&self) -> impl Iterator<Item = &AgentId> {
        self.steps.iter().filter_map(|s| s.agent.as_ref())
    }

    pub fn id(&self) -> &WorkflowId {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    pub fn scope(&self) -> &WorkflowScope {
        &self.scope
    }

    pub fn created_at(&self) -> SystemTime {
        self.created_at
    }

    pub fn updated_at(&self) -> SystemTime {
        self.updated_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENTED: &str = r#"
id: daily-report
name: Daily Report
steps:
  - id: research
    agent: research-agent
    input: "Find today's AI news"
  - id: summarize
    agent: writer-agent
    input: "Summarize: {{research.output}}"
    critical: true
    on_failure: continue
  - id: publish
    agent: publisher-agent
    input: "Send to #ai-news: {{ summarize.output }}"
    requires_approval: true
"#;

    #[test]
    fn parses_and_round_trips() {
        let workflow = Workflow::from_yaml(DOCUMENTED).unwrap();
        assert_eq!(workflow.steps().len(), 3);
        assert!(workflow.steps()[1].critical);
        assert_eq!(workflow.steps()[1].on_failure, FailurePolicy::Continue);
        assert!(workflow.steps()[2].requires_approval);
        assert_eq!(
            workflow.steps()[2].references().collect::<Vec<_>>(),
            ["summarize"]
        );
        let again = Workflow::from_yaml(&workflow.to_yaml()).unwrap();
        assert_eq!(again, workflow);
        // camelCase spellings from earlier versions still parse
        assert!(Workflow::from_yaml(
            "id: w\nname: W\nsteps:\n  - {id: a, agent: x, input: hi, requiresApproval: true}\n"
        )
        .unwrap()
        .steps()[0]
            .requires_approval);
    }

    #[test]
    fn provides_is_optional_and_round_trips() {
        let without = Workflow::from_yaml(DOCUMENTED).unwrap();
        assert!(without.provides().is_empty());
        assert!(!without.to_yaml().contains("provides"));

        let yaml = format!("{DOCUMENTED}provides: [daily-report.ai, news]\n");
        let with = Workflow::from_yaml(&yaml).unwrap();
        assert_eq!(with.provides(), ["daily-report.ai", "news"]);
        assert_eq!(Workflow::from_yaml(&with.to_yaml()).unwrap(), with);
        assert!(!with.same_definition(&without));

        for (bad, expected) in [
            ("provides: [Bad_Id]", "invalid capability id"),
            ("provides: [a, a]", "twice"),
        ] {
            let error = Workflow::from_yaml(&format!("{DOCUMENTED}{bad}\n"))
                .unwrap_err()
                .to_string();
            assert!(error.contains(expected), "{bad}: {error}");
        }
    }

    #[test]
    fn description_is_optional_and_round_trips() {
        let without = Workflow::from_yaml(DOCUMENTED).unwrap();
        assert_eq!(without.description(), None);
        assert!(!without.to_yaml().contains("description"));

        let yaml = "id: w\nname: W\ndescription: '  Fetches and reports.  '\nsteps:\n  - {id: a, agent: x, input: go}\n";
        let with = Workflow::from_yaml(yaml).unwrap();
        assert_eq!(with.description(), Some("Fetches and reports."));
        let again = Workflow::from_yaml(&with.to_yaml()).unwrap();
        assert_eq!(again.description(), Some("Fetches and reports."));
        // Blank reads as none
        let blank = Workflow::from_yaml(
            "id: w\nname: W\ndescription: ' '\nsteps:\n  - {id: a, agent: x, input: go}\n",
        )
        .unwrap();
        assert_eq!(blank.description(), None);
    }

    #[test]
    fn detects_cycles_and_dangling_steps() {
        let self_ref = "id: w\nname: W\nsteps:\n  - {id: a, agent: x, input: '{{a.output}}'}\n";
        assert!(matches!(
            Workflow::from_yaml(self_ref),
            Err(DomainError::Cycle { .. })
        ));
        let forward = "id: w\nname: W\nsteps:\n  - {id: a, agent: x, input: '{{b.output}}'}\n  - {id: b, agent: x, input: '{{a.output}}'}\n";
        assert!(matches!(
            Workflow::from_yaml(forward),
            Err(DomainError::Cycle { .. })
        ));
        let missing = "id: w\nname: W\nsteps:\n  - {id: a, agent: x, input: '{{zzz.output}}'}\n";
        assert!(matches!(
            Workflow::from_yaml(missing),
            Err(DomainError::DanglingReference { .. })
        ));
    }

    #[test]
    fn validate_step_graph_finds_missing_agents() {
        let workflow = Workflow::from_yaml(DOCUMENTED).unwrap();
        let known = ["research-agent", "writer-agent", "publisher-agent"];
        assert!(workflow
            .validate_step_graph(|a| known.contains(&a.as_str()))
            .is_ok());
        let error = workflow
            .validate_step_graph(|a| a.as_str() != "writer-agent")
            .unwrap_err();
        assert!(
            error.to_string().contains("agent 'writer-agent'"),
            "{error}"
        );
    }

    #[test]
    fn rejects_invalid_definitions() {
        for (yaml, expected) in [
            ("id: w\nname: ' '\nsteps: [{id: a, agent: x, input: hi}]", "name"),
            ("id: w\nname: W\nsteps: []", "steps"),
            ("id: w\nname: W\nsteps: [{id: a, agent: x, input: ' '}]", "no input"),
            ("id: w\nname: W\nsteps: [{id: a, agent: x, input: hi}, {id: a, agent: x, input: hi}]", "Duplicate"),
            ("id: w\nname: W\nsteps: [{id: a, agent: Bad_Agent, input: hi}]", "agent id"),
            ("id: ../w\nname: W\nsteps: [{id: a, agent: x, input: hi}]", "workflow id"),
            ("id: w\nname: W\nsteps: [{id: a, agent: x, input: hi, typo: 1}]", "unknown field"),
        ] {
            let error = Workflow::from_yaml(yaml).unwrap_err().to_string();
            assert!(error.contains(expected), "{yaml}: {error}");
        }
    }
}
