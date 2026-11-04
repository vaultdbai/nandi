//! Bundled workflow templates (Automations › New workflow › From a
//! template): two-step pipelines a new user can create without writing
//! YAML. Each agent step gets an agent the user picks; the result goes
//! through the same validation and repository as an imported file.

use std::collections::HashMap;

use serde::Serialize;
use specta::Type;

use crate::services::domain::workflow::{FailurePolicy, Step, StepType};
use crate::services::domain::{self, AgentId, StepId, WorkflowId};

/// One step of a template
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TemplateStep {
    pub id: String,
    /// An agent step: the user picks the agent
    pub needs_agent: bool,
    /// Pauses for the user's approval first
    pub requires_approval: bool,
    /// Done by a person
    pub manual: bool,
}

/// A template, named by ID (the UI words come from the locale files)
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowTemplate {
    pub id: String,
    /// English name, used when the user keeps the default
    pub name: String,
    pub steps: Vec<TemplateStep>,
}

struct Template {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    steps: &'static [TemplateStepDef],
}

struct TemplateStepDef {
    id: &'static str,
    /// The agent's message, or a manual step's instructions
    text: &'static str,
    manual: bool,
    requires_approval: bool,
}

const fn agent(id: &'static str, text: &'static str) -> TemplateStepDef {
    TemplateStepDef {
        id,
        text,
        manual: false,
        requires_approval: false,
    }
}

const TEMPLATES: &[Template] = &[
    Template {
        id: "daily-summary",
        name: "Daily summary",
        description: "An agent summarizes the day; you read it and follow up.",
        steps: &[
            agent(
                "summarize",
                "Write a short summary of today's work and list anything that needs my attention.",
            ),
            TemplateStepDef {
                id: "follow-up",
                text: "Read the summary and follow up on anything that needs you:\n\n{{summarize.output}}",
                manual: true,
                requires_approval: false,
            },
        ],
    },
    Template {
        id: "review-new-files",
        name: "Review new files",
        description: "An agent looks at new files and another writes a short report.",
        steps: &[
            agent(
                "analyze",
                "Look at the newest files in your workspace and list what changed and why it matters.",
            ),
            agent(
                "report",
                "Write a short report from this analysis:\n\n{{analyze.output}}",
            ),
        ],
    },
    Template {
        id: "approve-then-send",
        name: "Approve then send",
        description: "An agent drafts, you approve, then an agent sends it.",
        steps: &[
            agent("draft", "Draft a short status update about this week's work."),
            TemplateStepDef {
                id: "send",
                text: "Send this update:\n\n{{draft.output}}",
                manual: false,
                requires_approval: true,
            },
        ],
    },
    Template {
        id: "two-agent-pipeline",
        name: "Two-agent pipeline",
        description: "One agent researches, another writes it up.",
        steps: &[
            agent(
                "research",
                "Research the topic and list the key facts with their sources.",
            ),
            agent(
                "write",
                "Write a clear one-page summary from these notes:\n\n{{research.output}}",
            ),
        ],
    },
];

pub fn list() -> Vec<WorkflowTemplate> {
    TEMPLATES
        .iter()
        .map(|t| WorkflowTemplate {
            id: t.id.to_string(),
            name: t.name.to_string(),
            steps: t
                .steps
                .iter()
                .map(|s| TemplateStep {
                    id: s.id.to_string(),
                    needs_agent: !s.manual,
                    requires_approval: s.requires_approval,
                    manual: s.manual,
                })
                .collect(),
        })
        .collect()
}

/// A workflow ID from a name: lowercase letters, digits and hyphens,
/// suffixed `-2`, `-3`, … until it's free
pub fn id_for(name: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut base: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while base.contains("--") {
        base = base.replace("--", "-");
    }
    let base = base.trim_matches('-');
    let base = if base.is_empty() { "workflow" } else { base };
    if !taken(base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
        .expect("an unused suffix exists")
}

/// Builds template `template_id` as workflow `id` named `name`, with each
/// agent step's agent from `agents` (step ID → agent ID). Pure; the caller
/// saves it.
pub fn build(
    template_id: &str,
    id: &str,
    name: &str,
    agents: &HashMap<String, String>,
) -> Result<domain::Workflow, String> {
    let template = TEMPLATES
        .iter()
        .find(|t| t.id == template_id)
        .ok_or_else(|| format!("No workflow template '{template_id}'"))?;
    let steps = template
        .steps
        .iter()
        .map(|def| {
            let id = StepId::parse(def.id).map_err(|e| e.to_string())?;
            if def.manual {
                return Ok(Step {
                    id,
                    step_type: StepType::Manual,
                    agent: None,
                    input_template: String::new(),
                    instructions: Some(def.text.to_string()),
                    deadline_hours: None,
                    requires_approval: false,
                    critical: false,
                    on_failure: FailurePolicy::default(),
                    ask: None,
                });
            }
            let agent = agents
                .get(def.id)
                .filter(|a| !a.trim().is_empty())
                .ok_or_else(|| format!("Choose an agent for step '{}'", def.id))?;
            Ok(Step {
                id,
                step_type: StepType::Agent,
                agent: Some(AgentId::parse(agent.trim()).map_err(|e| e.to_string())?),
                input_template: def.text.to_string(),
                instructions: None,
                deadline_hours: None,
                requires_approval: def.requires_approval,
                critical: false,
                on_failure: FailurePolicy::default(),
                ask: None,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let name = if name.trim().is_empty() {
        template.name
    } else {
        name.trim()
    };
    domain::Workflow::new(
        WorkflowId::parse(id).map_err(|e| e.to_string())?,
        name,
        steps,
    )
    .map(|w| w.with_description(Some(template.description.to_string())))
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agents(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(s, a)| (s.to_string(), a.to_string()))
            .collect()
    }

    #[test]
    fn every_template_builds_a_valid_two_step_workflow() {
        for template in list() {
            let bound = template
                .steps
                .iter()
                .filter(|s| s.needs_agent)
                .map(|s| (s.id.clone(), "writer".to_string()))
                .collect();
            let workflow = build(&template.id, "w", "", &bound).unwrap();
            assert_eq!(workflow.steps().len(), 2, "{}", template.id);
            assert_eq!(workflow.name(), template.name);
            assert!(workflow.description().is_some());
            // What's saved parses back the same
            let again = domain::Workflow::from_yaml(&workflow.to_yaml()).unwrap();
            assert_eq!(again.steps(), workflow.steps());
        }
    }

    #[test]
    fn an_agent_step_needs_an_agent() {
        let error = build(
            "two-agent-pipeline",
            "w",
            "W",
            &agents(&[("research", "a")]),
        )
        .unwrap_err();
        assert!(error.contains("'write'"), "{error}");
        assert!(build("nope", "w", "W", &HashMap::new()).is_err());
        assert!(build(
            "two-agent-pipeline",
            "w",
            "W",
            &agents(&[("research", "Bad Id"), ("write", "a")])
        )
        .is_err());
    }

    #[test]
    fn approval_and_manual_steps_come_through() {
        let send = build(
            "approve-then-send",
            "w",
            "Mine",
            &agents(&[("draft", "a"), ("send", "b")]),
        )
        .unwrap();
        assert!(send.steps()[1].requires_approval);
        assert_eq!(send.name(), "Mine");
        let daily = build("daily-summary", "w", "", &agents(&[("summarize", "a")])).unwrap();
        assert!(daily.steps()[1].is_manual());
    }

    #[test]
    fn ids_come_from_the_name_and_stay_unique() {
        assert_eq!(id_for("Daily Summary!", |_| false), "daily-summary");
        assert_eq!(id_for("  ", |_| false), "workflow");
        let taken = ["daily-summary", "daily-summary-2"];
        assert_eq!(
            id_for("Daily summary", |id| taken.contains(&id)),
            "daily-summary-3"
        );
    }
}
