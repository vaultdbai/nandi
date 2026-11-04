//! The gateway's view of this instance for one ask (Track A): what's
//! running, what finished recently, capabilities (owned / provided /
//! used), agents, workflows, policies, and network members, plus the
//! artifact the ask is about. Loaded once per ask and shared by the
//! classifier, the answerer, the planner, and the editor.

use serde::{Deserialize, Serialize};

use super::{ArtifactRef, AskContext};
use crate::model::{RunStatus, RunSummary};
use crate::services::AppState;

/// Runs listed as recent outputs
const RECENT_OUTPUTS: usize = 5;
/// How far back recent outputs go
const RECENT_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1000;
/// Characters kept of a run's input or output
const EXCERPT_CHARS: usize = 240;
/// Characters of the focused artifact's file shown to the model
const FOCUSED_CHARS: usize = 6000;

/// Where the ask was typed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "camelCase")]
pub enum ViewScope {
    Home,
    Agent(String),
    Capability(String),
    Workflow(String),
    Member(String),
    /// Studio, Work, the companion, …
    Other(String),
}

impl ViewScope {
    /// `home`, `agent:<id>`, `capability:<id>`, `workflow:<id>`,
    /// `member:<instance>`; anything else is `Other`
    pub fn parse(view: Option<&str>) -> Self {
        let Some(view) = view.map(str::trim).filter(|v| !v.is_empty()) else {
            return Self::Home;
        };
        match view.split_once(':') {
            Some(("agent", id)) => Self::Agent(id.to_string()),
            Some(("capability", id)) => Self::Capability(id.to_string()),
            Some(("workflow", id)) => Self::Workflow(id.to_string()),
            Some(("member", id)) => Self::Member(id.to_string()),
            _ if view == "home" || view == "dashboard" => Self::Home,
            _ => Self::Other(view.to_string()),
        }
    }

    /// The artifact a view is about, when the request didn't name one
    pub fn artifact(&self) -> Option<ArtifactRef> {
        match self {
            Self::Agent(id) => Some(ArtifactRef::Agent(id.clone())),
            Self::Capability(id) => Some(ArtifactRef::Capability(id.clone())),
            Self::Workflow(id) => Some(ArtifactRef::Workflow(id.clone())),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunBrief {
    pub id: String,
    pub agent_id: String,
    pub status: String,
    pub input: String,
    /// The final reply (or the error), once ended
    pub output: Option<String>,
    pub label: Option<String>,
    pub started_at: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityBrief {
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub owned: bool,
    pub provided: bool,
    pub used: bool,
    pub online_providers: u32,
    pub cost_per_call_usd: Option<f64>,
    pub sla_seconds: u32,
    pub approval_required: bool,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentBrief {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowBrief {
    pub id: String,
    pub name: String,
    /// Agent IDs, in step order
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PolicyBrief {
    pub id: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemberBrief {
    pub instance_id: String,
    pub name: String,
    pub online: bool,
    pub accepts_delegation: bool,
    pub capabilities: Vec<String>,
}

/// Something waiting on the owner, or a handoff they should know about
/// (wave EF)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitingBrief {
    /// `manual_step` or `reassigned`
    pub kind: String,
    /// The workflow run or task ID
    pub id: String,
    /// What needs them (a workflow's name, a task's capability)
    pub title: String,
    pub detail: String,
}

/// The artifact the ask is about, as it is on disk
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FocusedArtifact {
    pub artifact: ArtifactRef,
    /// The file (truncated), or `None` when it couldn't be read
    pub content: Option<String>,
}

/// Everything the gateway knows that an ask may draw on
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayContext {
    pub view: ViewScope,
    pub focused: Option<FocusedArtifact>,
    pub active_runs: Vec<RunBrief>,
    pub recent_outputs: Vec<RunBrief>,
    pub capabilities: Vec<CapabilityBrief>,
    pub agents: Vec<AgentBrief>,
    pub workflows: Vec<WorkflowBrief>,
    pub policies: Vec<PolicyBrief>,
    pub members: Vec<MemberBrief>,
    /// Manual workflow steps waiting on the owner, and recent reassignments
    pub waiting_on_you: Vec<WaitingBrief>,
}

/// What the loader read, before it's scoped to the ask
#[derive(Debug, Clone, Default)]
pub struct ContextSources {
    pub runs: Vec<RunSummary>,
    pub capabilities: Vec<CapabilityBrief>,
    pub agents: Vec<AgentBrief>,
    pub workflows: Vec<WorkflowBrief>,
    pub policies: Vec<PolicyBrief>,
    pub members: Vec<MemberBrief>,
    pub waiting: Vec<WaitingBrief>,
}

/// Reads everything an ask may draw on. Sources that are unavailable
/// (no gateway, no capability service) are left empty.
pub async fn load(state: &AppState, ask: &AskContext) -> GatewayContext {
    let sources = read_sources(state).await;
    let view = ViewScope::parse(ask.current_view.as_deref());
    let artifact = ask.focused_artifact.clone().or_else(|| view.artifact());
    let focused = match artifact {
        Some(artifact) => {
            let content = crate::services::studio::artifact_loader::read_raw(state, &artifact)
                .ok()
                .map(|text| truncate(&text, FOCUSED_CHARS));
            Some(FocusedArtifact { artifact, content })
        }
        None => None,
    };
    build(sources, view, focused, &ask.recent_runs)
}

/// Scopes the sources to the ask: runs the user was looking at come first
pub fn build(
    sources: ContextSources,
    view: ViewScope,
    focused: Option<FocusedArtifact>,
    recent_runs: &[String],
) -> GatewayContext {
    let (mut active, mut ended): (Vec<RunSummary>, Vec<RunSummary>) =
        sources.runs.into_iter().partition(|run| {
            matches!(
                run.status,
                RunStatus::Queued | RunStatus::Running | RunStatus::AwaitingApproval
            )
        });
    // Pinned (looked-at) runs first, then the newest
    let rank = |run: &RunSummary| !recent_runs.contains(&run.id);
    active.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then(b.started_at.total_cmp(&a.started_at))
    });
    ended.retain(|run| run.status == RunStatus::Completed || recent_runs.contains(&run.id));
    ended.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then(b.started_at.total_cmp(&a.started_at))
    });
    ended.truncate(RECENT_OUTPUTS);
    GatewayContext {
        view,
        focused,
        active_runs: active.iter().map(brief).collect(),
        recent_outputs: ended.iter().map(brief).collect(),
        capabilities: sources.capabilities,
        agents: sources.agents,
        workflows: sources.workflows,
        policies: sources.policies,
        members: sources.members,
        waiting_on_you: sources.waiting,
    }
}

fn brief(run: &RunSummary) -> RunBrief {
    RunBrief {
        id: run.id.clone(),
        agent_id: run.agent_id.clone(),
        status: run.status.as_str().to_string(),
        input: truncate(&run.input, EXCERPT_CHARS),
        output: run.summary.as_deref().map(|s| truncate(s, EXCERPT_CHARS)),
        label: run.source_label.clone(),
        started_at: run.started_at,
    }
}

pub fn truncate(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

async fn read_sources(state: &AppState) -> ContextSources {
    let since = crate::db::now_ms() - RECENT_WINDOW_MS;
    let runs = state
        .db()
        .call(move |conn| {
            let mut runs = crate::db::active_runs(conn)?;
            for run in crate::db::runs_since(conn, since)? {
                if !runs.iter().any(|r| r.id == run.id) {
                    runs.push(run);
                }
            }
            Ok(runs)
        })
        .await
        .unwrap_or_else(|e: String| {
            log::warn!("Ask context: runs not loaded: {e}");
            Vec::new()
        });
    let capabilities = match state.capabilities() {
        Some(service) => service
            .list_listings()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|listing| CapabilityBrief {
                id: listing.definition.id.clone(),
                name: listing.definition.name.clone(),
                version: listing.definition.version.clone(),
                description: truncate(&listing.definition.description, EXCERPT_CHARS),
                owned: listing.owned,
                provided: listing.provided,
                used: listing.used,
                online_providers: listing.online_providers,
                cost_per_call_usd: listing
                    .definition
                    .policy
                    .cost
                    .as_ref()
                    .and_then(|c| c.per_call_usd),
                sla_seconds: listing.definition.policy.sla_seconds,
                approval_required: listing.definition.policy.approval_required,
                tags: listing.definition.tags.clone(),
            })
            .collect(),
        None => Vec::new(),
    };
    let (agents, workflows) = state.store().read(|repo| {
        let agents = repo
            .agents()
            .map(|a| AgentBrief {
                id: a.id().to_string(),
                name: a.name().to_string(),
                description: truncate(a.description(), EXCERPT_CHARS),
            })
            .collect();
        let workflows = repo
            .workflows()
            .map(|w| WorkflowBrief {
                id: w.id().to_string(),
                name: w.name().to_string(),
                steps: w
                    .steps()
                    .iter()
                    .map(|s| s.performer().to_string())
                    .collect(),
            })
            .collect();
        (agents, workflows)
    });
    let mut policies = Vec::new();
    if let Some(gateway) = state.gateway() {
        if let Ok(manifest) = gateway.manifest() {
            policies.push(gateway_policy(&manifest.policy));
        }
    }
    for (id, text) in crate::services::studio::artifact_loader::policy_files(state) {
        policies.push(PolicyBrief {
            id,
            summary: truncate(&text, EXCERPT_CHARS),
        });
    }
    let known = crate::services::network::known_members(state.db());
    let members: Vec<MemberBrief> = known
        .iter()
        .map(|member| MemberBrief {
            instance_id: member.instance_id.clone(),
            name: member.display_name.clone(),
            online: member.online,
            accepts_delegation: member.gateway.accepts_delegation,
            capabilities: member.gateway.capabilities.clone(),
        })
        .collect();
    let mut waiting: Vec<WaitingBrief> =
        crate::services::workflows::manual_steps::pending(state.db())
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|step| WaitingBrief {
                kind: "manual_step".to_string(),
                id: step.run_id,
                title: step.workflow_name,
                detail: truncate(&step.instructions, EXCERPT_CHARS),
            })
            .collect();
    let name_of = |instance: &str| {
        known
            .iter()
            .find(|m| m.instance_id == instance)
            .map_or_else(|| instance.to_string(), |m| m.display_name.clone())
    };
    for (task_id, capability, to, reason) in recent_reassignments(state, since).await {
        let mut detail = format!("Reassigned to {}", name_of(&to));
        if let Some(reason) = reason {
            detail.push_str(&format!(": {}", truncate(&reason, EXCERPT_CHARS)));
        }
        waiting.push(WaitingBrief {
            kind: "reassigned".to_string(),
            id: task_id,
            title: capability,
            detail,
        });
    }
    ContextSources {
        runs,
        capabilities,
        agents,
        workflows,
        policies,
        members,
        waiting,
    }
}

/// Reassignments since `since`, newest first: (task, capability, to, reason)
async fn recent_reassignments(
    state: &AppState,
    since: i64,
) -> Vec<(String, String, String, Option<String>)> {
    state
        .db()
        .call(move |conn| {
            let mut stmt = conn.prepare(
                "SELECT r.task_id, COALESCE(t.capability, ''), r.to_instance, r.reason
                   FROM reassigned_tasks r LEFT JOIN tasks t ON t.id = r.task_id
                  WHERE r.reassigned_at >= ?1
                  ORDER BY r.reassigned_at DESC LIMIT 10",
            )?;
            let rows = stmt.query_map([since], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;
            rows.collect()
        })
        .await
        .unwrap_or_default()
}

fn gateway_policy(policy: &crate::services::gateway::GatewayPolicy) -> PolicyBrief {
    let mut summary = format!(
        "At most {} tasks at a time, {} requests per hour per caller",
        policy.max_concurrent, policy.rate_limit_per_hour
    );
    if !policy.require_approval.is_empty() {
        summary.push_str(&format!(
            "; approval needed for {}",
            policy.require_approval.join(", ")
        ));
    }
    if !policy.hide.is_empty() {
        summary.push_str(&format!("; hidden: {}", policy.hide.join(", ")));
    }
    PolicyBrief {
        id: "gateway".to_string(),
        summary,
    }
}

/// Studio's guides for an answer: the index, and the body of the guide
/// the question is most about (by words shared with its title and `when`),
/// so how-to answers come from the guides instead of guesses
pub fn describe_guides(question: &str) -> String {
    use crate::services::studio::guides;
    let words = |text: &str| -> std::collections::BTreeSet<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() > 3)
            .map(str::to_lowercase)
            .collect()
    };
    let asked = words(question);
    let best = guides::all()
        .iter()
        .map(|guide| {
            let about = words(&format!("{} {} {}", guide.id, guide.title, guide.when));
            (asked.intersection(&about).count(), guide)
        })
        .filter(|(shared, _)| *shared >= 2)
        .max_by_key(|(shared, _)| *shared)
        .map(|(_, guide)| guide);
    let mut out = format!(
        "\n## Guides (how to do things; say the user can ask for more in Ask)\n\n{}\n",
        guides::index_lines()
    );
    if let Some(guide) = best {
        out.push_str(&format!("\n## Guide: {}\n\n{}\n", guide.title, guide.body));
    }
    out
}

/// The context as the system prompt section the model reads
pub fn describe(context: &GatewayContext) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "## Where the owner is\n\n{}\n",
        describe_view(&context.view)
    ));
    if let Some(focused) = &context.focused {
        out.push_str(&format!(
            "\n## The artifact in focus: {}\n\n",
            artifact_label(&focused.artifact)
        ));
        match &focused.content {
            Some(content) => out.push_str(&format!("```\n{content}\n```\n")),
            None => out.push_str("(its file couldn't be read)\n"),
        }
    }
    let list = |title: &str, lines: Vec<String>| {
        if lines.is_empty() {
            format!("\n## {title}\n\n(none)\n")
        } else {
            format!("\n## {title}\n\n{}\n", lines.join("\n"))
        }
    };
    out.push_str(&list(
        "Waiting on the owner",
        context
            .waiting_on_you
            .iter()
            .map(|w| match w.kind.as_str() {
                "manual_step" => format!(
                    "- Workflow \"{}\" needs them (manual step, run `{}`): {}",
                    w.title, w.id, w.detail
                ),
                _ => format!("- Task `{}` ({}): {}", w.id, w.title, w.detail),
            })
            .collect(),
    ));
    out.push_str(&list(
        "Running now",
        context.active_runs.iter().map(describe_run).collect(),
    ));
    out.push_str(&list(
        "Recent outputs",
        context.recent_outputs.iter().map(describe_run).collect(),
    ));
    out.push_str(&list(
        "Capabilities",
        context
            .capabilities
            .iter()
            .map(|c| {
                let mut relation = Vec::new();
                if c.owned {
                    relation.push("owned");
                }
                if c.provided {
                    relation.push("provided here");
                }
                if c.used {
                    relation.push("used");
                }
                let relation = if relation.is_empty() {
                    "discoverable".to_string()
                } else {
                    relation.join(", ")
                };
                format!(
                    "- `{}` v{} ({}): {} [{relation}; {} online providers]",
                    c.id, c.version, c.name, c.description, c.online_providers
                )
            })
            .collect(),
    ));
    out.push_str(&list(
        "Agents",
        context
            .agents
            .iter()
            .map(|a| format!("- `{}` ({}): {}", a.id, a.name, a.description))
            .collect(),
    ));
    out.push_str(&list(
        "Workflows",
        context
            .workflows
            .iter()
            .map(|w| format!("- `{}` ({}): {}", w.id, w.name, w.steps.join(" → ")))
            .collect(),
    ));
    out.push_str(&list(
        "Policies",
        context
            .policies
            .iter()
            .map(|p| format!("- {}: {}", p.id, p.summary))
            .collect(),
    ));
    out.push_str(&list(
        "Network members",
        context
            .members
            .iter()
            .map(|m| {
                format!(
                    "- `{}` ({}): {}{}; offers {}",
                    m.instance_id,
                    m.name,
                    if m.online { "online" } else { "offline" },
                    if m.accepts_delegation {
                        ", takes requests"
                    } else {
                        ""
                    },
                    if m.capabilities.is_empty() {
                        "nothing listed".to_string()
                    } else {
                        m.capabilities.join(", ")
                    }
                )
            })
            .collect(),
    ));
    out
}

fn describe_view(view: &ViewScope) -> String {
    match view {
        ViewScope::Home => "The home dashboard".to_string(),
        ViewScope::Agent(id) => format!("Agent `{id}`'s page"),
        ViewScope::Capability(id) => format!("Capability `{id}`'s page"),
        ViewScope::Workflow(id) => format!("Workflow `{id}`'s page"),
        ViewScope::Member(id) => format!("Network member `{id}`'s profile"),
        ViewScope::Other(view) => format!("The {view} view"),
    }
}

fn describe_run(run: &RunBrief) -> String {
    let label = run
        .label
        .as_deref()
        .map(|l| format!(" ({l})"))
        .unwrap_or_default();
    match &run.output {
        Some(output) => format!(
            "- run `{}` by `{}`{label}, {}: asked \"{}\" → \"{}\"",
            run.id, run.agent_id, run.status, run.input, output
        ),
        None => format!(
            "- run `{}` by `{}`{label}, {}: \"{}\"",
            run.id, run.agent_id, run.status, run.input
        ),
    }
}

/// `agent writer`, `capability code-review.python`, `NANDI.md`, …
pub fn artifact_label(artifact: &ArtifactRef) -> String {
    match artifact {
        ArtifactRef::Capability(id) => format!("capability `{id}`"),
        ArtifactRef::Agent(id) => format!("agent `{id}`"),
        ArtifactRef::Workflow(id) => format!("workflow `{id}`"),
        ArtifactRef::Policy(id) => format!("policy `{id}`"),
        ArtifactRef::Gateway => "the gateway manifest (NANDI.md)".to_string(),
    }
}
