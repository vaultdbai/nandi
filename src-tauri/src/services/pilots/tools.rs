//! Which pilot tools an agent may use, and running them.
//!
//! An agent reaches a pilot only when all of these hold:
//! 1. its skill.md `pilots:` block lists the pilot's ID (e.g. `opendesk`);
//! 2. the pilot is installed and running in this app session;
//! 3. the user granted the agent every capability the pilot exposes
//!    (the Pilots consent dialog; stored per agent);
//! 4. its last health check passed.
//!
//! The host's orchestrator is the MCP client, so a grant for one pilot
//! never makes another reachable. Pilot tools reach the model as
//! `pilot_<pilot_id>__<tool>` and need approval unless the pilot marks them
//! `readOnlyHint`.

use std::collections::HashSet;
use std::time::Instant;

use serde_json::Value;

use super::audit::{self, PilotAuditEntry};
use super::mcp::PilotSession;
use super::store::{self, PersistedPilot};
use super::{browser, handover};
use super::{catalog, runtime, PilotDefinition};
use crate::db::Db;
use crate::engine::ToolOutcome;
use crate::llm::ToolSpec;
use crate::services::domain::policy::Refusal;
use crate::services::domain::Agent;
use crate::services::events::EventBus;

pub const PREFIX: &str = "pilot_";
const SEPARATOR: &str = "__";

/// The model-facing name of a pilot's tool.
pub fn tool_name(pilot_id: &str, tool: &str) -> String {
    format!("{PREFIX}{pilot_id}{SEPARATOR}{tool}")
}

/// `(pilot_id, tool)` for a name built by [`tool_name`].
pub fn parse_tool_name(name: &str) -> Option<(&str, &str)> {
    let (pilot, tool) = name.strip_prefix(PREFIX)?.split_once(SEPARATOR)?;
    (!pilot.is_empty() && !tool.is_empty()).then_some((pilot, tool))
}

pub fn is_pilot_tool(name: &str) -> bool {
    parse_tool_name(name).is_some()
}

/// The pilot IDs the agent's skill.md declares.
pub fn declared(agent: &Agent) -> Vec<String> {
    agent
        .capabilities()
        .pilots()
        .iter()
        .map(|p| p.as_str().to_string())
        .collect()
}

/// Whether an agent may use a pilot right now, and if not, why.
#[derive(Debug, Clone, PartialEq)]
pub enum Access {
    Allowed,
    NotDeclared,
    NotInstalled,
    NotRunning,
    /// The capabilities still waiting for the user's consent
    NeedsConsent(Vec<String>),
    Unhealthy,
}

impl Access {
    pub fn reason(&self, pilot: &PilotDefinition) -> String {
        let name = &pilot.name;
        match self {
            Access::Allowed => format!("{name} is available"),
            Access::NotDeclared => format!(
                "This agent's skill.md doesn't list the pilot '{}'",
                pilot.id
            ),
            Access::NotInstalled => format!("{name} isn't installed. Install it in Pilots."),
            Access::NotRunning => format!("{name} isn't running. Start it in Pilots."),
            Access::NeedsConsent(missing) => format!(
                "{name} needs your consent for this agent ({}). Allow it in Pilots.",
                missing.join(", ")
            ),
            Access::Unhealthy => {
                format!("{name} is unhealthy (its health check failed); its tools are unavailable")
            }
        }
    }
}

/// The access rule itself (no I/O).
pub fn access(
    declared: &[String],
    pilot: &PilotDefinition,
    state: &PersistedPilot,
    granted: &[String],
) -> Access {
    if !declared.iter().any(|d| d == &pilot.id) {
        return Access::NotDeclared;
    }
    if !state.installed {
        return Access::NotInstalled;
    }
    if !state.is_live() {
        return Access::NotRunning;
    }
    let missing: Vec<String> = pilot
        .capabilities
        .iter()
        .filter(|c| !granted.contains(c))
        .cloned()
        .collect();
    if !missing.is_empty() {
        return Access::NeedsConsent(missing);
    }
    if !state.healthy {
        return Access::Unhealthy;
    }
    Access::Allowed
}

async fn check(
    db: &Db,
    agent_id: &str,
    declared: &[String],
    pilot: &PilotDefinition,
) -> Result<Access, String> {
    let (agent, id) = (agent_id.to_string(), pilot.id.clone());
    let (state, granted) = db
        .call(move |conn| Ok((store::load(conn, &id)?, store::granted(conn, &agent, &id)?)))
        .await?;
    Ok(access(declared, pilot, &state, &granted))
}

/// The pilot tools offered to the model this step, and why any declared
/// pilot was left out.
#[derive(Debug, Default)]
pub struct PilotToolset {
    pub tools: Vec<ToolSpec>,
    read_only: HashSet<String>,
    /// Browser pilots the agent uses, with what its skill.md allows
    browsers: std::collections::HashMap<String, BrowserRules>,
    /// The agent these tools are for
    agent: String,
    /// Controls its policies refuse in a browser (`browser.refuse`)
    refusals: Vec<Refusal>,
    /// One per declared pilot that isn't usable (for `agent://warning`)
    pub warnings: Vec<String>,
    /// For the system prompt: what each browser may type (skill.md's
    /// values, and secrets by name). Tool descriptions alone weren't
    /// enough: the model treated them as docs, not as its own data
    pub prompt_notes: Vec<String>,
    /// `(pilot_id, capabilities not granted)` for each declared, running
    /// pilot left out only for lack of consent (`pilot://consent_needed`)
    pub consent_needed: Vec<(String, Vec<String>)>,
}

/// What a browser call may do without asking, for one agent
#[derive(Debug, Clone, Default)]
struct BrowserRules {
    options: Option<crate::services::domain::capability::PilotOptions>,
    secret_names: Vec<String>,
}

impl PilotToolset {
    /// The agent's `browser.refuse` rules (from its narrowed skill.md)
    pub fn set_refusals(&mut self, refusals: Vec<Refusal>) {
        self.refusals = refusals;
    }

    /// Every pilot tool needs approval unless its pilot marks it read-only.
    /// A browser decides per call: reading and moving between allowed
    /// pages don't ask; the rest asks unless skill.md allows it
    /// (`browser::needs_approval`); a call a policy refuses doesn't ask, it's
    /// refused when it runs. A pilot-shaped name the set doesn't know (e.g.
    /// hallucinated) asks.
    pub fn requires_approval(&self, name: &str, args: &Value) -> bool {
        let Some((pilot, tool)) = parse_tool_name(name) else {
            return false;
        };
        match self.browsers.get(pilot) {
            Some(rules) => {
                let action = browser::action(
                    tool,
                    args,
                    self.read_only.contains(name),
                    &rules.secret_names,
                );
                if browser::refused_before(&self.agent, pilot, &self.refusals, tool, args, action) {
                    return false;
                }
                browser::needs_approval(action, rules.options.as_ref())
            }
            None => !self.read_only.contains(name),
        }
    }
}

/// Lists the tools of every pilot `agent` may use. A pilot whose server
/// doesn't answer is marked unhealthy and skipped.
pub async fn for_agent(bus: &EventBus, agent: &Agent) -> PilotToolset {
    let declared = declared(agent);
    let mut set = PilotToolset {
        agent: agent.id().as_str().to_string(),
        ..PilotToolset::default()
    };
    for pilot_id in &declared {
        let pilot = match catalog::find(pilot_id) {
            Ok(pilot) => pilot,
            Err(e) => {
                set.warnings.push(e);
                continue;
            }
        };
        match check(&bus.db, agent.id().as_str(), &declared, &pilot).await {
            Ok(Access::Allowed) => {}
            Ok(denied) => {
                set.warnings.push(denied.reason(&pilot));
                if let Access::NeedsConsent(missing) = denied {
                    set.consent_needed.push((pilot.id.clone(), missing));
                }
                continue;
            }
            Err(e) => {
                set.warnings.push(format!("{}: {e}", pilot.name));
                continue;
            }
        }
        let listed = if browser::is_browser(&pilot.id) {
            let options = agent.capabilities().pilot_options(&pilot.id).cloned();
            set.browsers.insert(
                pilot.id.clone(),
                BrowserRules {
                    secret_names: options
                        .as_ref()
                        .map(|o| o.secrets.keys().cloned().collect())
                        .unwrap_or_default(),
                    options,
                },
            );
            browser::tools(agent, &pilot.id).await
        } else {
            list(&pilot).await
        };
        // What a browser may type: skill.md's values, and secrets by name
        let fill_note = agent
            .capabilities()
            .pilot_options(&pilot.id)
            .filter(|_| browser::is_browser(&pilot.id))
            .map(|options| {
                let mut note = String::new();
                if !options.values.is_empty() {
                    let values: Vec<String> = options
                        .values
                        .iter()
                        .map(|(k, v)| format!("{k}: {v}"))
                        .collect();
                    note.push_str(&format!(" Values to fill in: {}.", values.join("; ")));
                }
                if !options.secrets.is_empty() {
                    let names: Vec<&str> = options.secrets.keys().map(String::as_str).collect();
                    note.push_str(&format!(
                        " Secrets: type the name ({}) as the text and the browser fills in the value; you never see it.",
                        names.join(", ")
                    ));
                }
                note
            })
            .unwrap_or_default();
        if !fill_note.is_empty() && listed.is_ok() {
            set.prompt_notes
                .push(format!("{} ({}):{fill_note}", pilot.name, pilot.id));
        }
        match listed {
            Ok(tools) => {
                for tool in tools {
                    let name = tool_name(&pilot.id, &tool.spec.name);
                    if tool.read_only {
                        set.read_only.insert(name.clone());
                    }
                    // Also on navigate, the tool a browser task starts with:
                    // the typing tools may be found only later, through
                    // tool search, after the agent has planned without them
                    let fills = matches!(
                        tool.spec.name.as_str(),
                        "browser_type" | "browser_fill_form" | "browser_navigate"
                    );
                    set.tools.push(ToolSpec {
                        name,
                        description: format!(
                            "[{}] {}{}",
                            pilot.name,
                            tool.spec.description,
                            if fills { fill_note.as_str() } else { "" }
                        ),
                        input_schema: tool.spec.input_schema,
                    });
                }
            }
            Err(e) => {
                runtime::set_healthy(bus, &pilot.id, false).await;
                set.warnings.push(format!(
                    "{} is unhealthy ({e}); its tools are unavailable",
                    pilot.name
                ));
            }
        }
    }
    set
}

/// The tools a running pilot's server offers (also the tool catalog's
/// source for pilot tools)
pub async fn list(pilot: &PilotDefinition) -> Result<Vec<super::mcp::PilotTool>, String> {
    if browser::is_browser(&pilot.id) {
        // One server per agent: its tools are listed per agent
        return Ok(Vec::new());
    }
    let session = PilotSession::connect(pilot.mcp_port).await?;
    let tools = session.list_tools().await;
    session.close().await;
    tools
}

/// Runs a `pilot_<id>__<tool>` call (`call_id`) for `agent`, re-checking
/// access first (consent or health may have changed since the step was
/// planned). Every call is audited. Failures are tool errors the model sees.
pub async fn call(
    bus: &EventBus,
    agent: &Agent,
    call_id: &str,
    name: &str,
    args: &Value,
) -> ToolOutcome {
    let Some((pilot_id, tool)) = parse_tool_name(name) else {
        return ToolOutcome {
            result: format!("'{name}' is not a pilot tool"),
            is_error: true,
        };
    };
    let mut entry = PilotAuditEntry::new(agent.id().as_str(), pilot_id, tool, args);
    let started = Instant::now();
    let result = run(bus, agent, call_id, pilot_id, tool, args).await;
    entry.duration_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
    let outcome = match result {
        Ok((content, is_error)) => {
            entry.outcome = if is_error { "error" } else { "ok" }.to_string();
            ToolOutcome {
                result: content,
                is_error,
            }
        }
        Err((message, kind)) => {
            entry.outcome = kind.to_string();
            ToolOutcome {
                result: format!("Tool '{name}' is not available: {message}"),
                is_error: true,
            }
        }
    };
    audit::record(&entry).await;
    outcome
}

/// `Err((why, "denied" | "error"))`
async fn run(
    bus: &EventBus,
    agent: &Agent,
    call_id: &str,
    pilot_id: &str,
    tool: &str,
    args: &Value,
) -> Result<(String, bool), (String, &'static str)> {
    let pilot = catalog::find(pilot_id).map_err(|e| (e, "denied"))?;
    let declared = declared(agent);
    match check(&bus.db, agent.id().as_str(), &declared, &pilot).await {
        Ok(Access::Allowed) => {}
        Ok(denied) => return Err((denied.reason(&pilot), "denied")),
        Err(e) => return Err((e, "error")),
    }
    // Approved means the person answered: the hand-over's result is theirs
    if handover::is_hand_over(&tool_name(&pilot.id, tool)) {
        return Ok((handover::result(handover::take(call_id), args), false));
    }
    if browser::is_browser(&pilot.id) {
        return browser::call(agent, &pilot.id, tool, args)
            .await
            .map_err(|e| (e, "error"));
    }
    let session = match PilotSession::connect(pilot.mcp_port).await {
        Ok(session) => session,
        Err(e) => {
            runtime::set_healthy(bus, &pilot.id, false).await;
            return Err((format!("{} didn't answer: {e}", pilot.name), "error"));
        }
    };
    let result = session.call(tool, args).await;
    session.close().await;
    result.map_err(|e| (e, "error"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pilot() -> PilotDefinition {
        catalog::find("opendesk").unwrap()
    }

    fn live(healthy: bool) -> PersistedPilot {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        store::save_installed(&conn, "opendesk", true).unwrap();
        store::save_running(&conn, "opendesk", Some(1), 17899).unwrap();
        store::save_healthy(&conn, "opendesk", healthy).unwrap();
        store::load(&conn, "opendesk").unwrap()
    }

    #[test]
    fn names_round_trip() {
        let name = tool_name("desktop-pilot", "click__twice");
        assert_eq!(name, "pilot_desktop-pilot__click__twice");
        assert_eq!(
            parse_tool_name(&name),
            Some(("desktop-pilot", "click__twice"))
        );
        for not in ["shell", "pilot_", "pilot_x", "pilot___t", "pilot_x__"] {
            assert_eq!(parse_tool_name(not), None, "{not}");
        }
    }

    #[test]
    fn consent_is_enforced_per_capability() {
        let declared = vec!["opendesk".to_string()];
        let all: Vec<String> = pilot().capabilities.clone();
        let some = vec![all[0].clone()];

        assert_eq!(
            access(&declared, &pilot(), &live(true), &[]),
            Access::NeedsConsent(all.clone())
        );
        assert_eq!(
            access(&declared, &pilot(), &live(true), &some),
            Access::NeedsConsent(all[1..].to_vec())
        );
        assert_eq!(
            access(&declared, &pilot(), &live(true), &all),
            Access::Allowed
        );
        // Consent doesn't stand in for the skill.md declaration
        assert_eq!(
            access(&[], &pilot(), &live(true), &all),
            Access::NotDeclared
        );
        assert_eq!(
            access(&["other-pilot".to_string()], &pilot(), &live(true), &all),
            Access::NotDeclared
        );
    }

    #[test]
    fn only_running_healthy_pilots_route() {
        let declared = vec!["opendesk".to_string()];
        let all = pilot().capabilities.clone();
        assert_eq!(
            access(&declared, &pilot(), &PersistedPilot::default(), &all),
            Access::NotInstalled
        );
        let installed = PersistedPilot {
            installed: true,
            ..Default::default()
        };
        assert_eq!(
            access(&declared, &pilot(), &installed, &all),
            Access::NotRunning
        );
        assert_eq!(
            access(&declared, &pilot(), &live(false), &all),
            Access::Unhealthy
        );
    }

    #[test]
    fn pilot_tools_need_approval_unless_read_only() {
        let mut set = PilotToolset::default();
        let args = serde_json::json!({});
        set.read_only.insert(tool_name("opendesk", "screenshot"));
        assert!(set.requires_approval("pilot_opendesk__click", &args));
        assert!(!set.requires_approval("pilot_opendesk__screenshot", &args));
        assert!(set.requires_approval("pilot_unknown__anything", &args));
        // Runtime tools follow skill.md, not this
        assert!(!set.requires_approval("shell", &args));

        // A browser decides per call, by what skill.md allows
        set.browsers
            .insert(browser::HEADLESS.into(), BrowserRules::default());
        let click = tool_name(browser::HEADLESS, "browser_click");
        let go = tool_name(browser::HEADLESS, "browser_navigate");
        let next = serde_json::json!({"element": "Next link"});
        assert!(!set.requires_approval(&go, &serde_json::json!({"url": "https://a"})));
        assert!(set.requires_approval(&click, &next));
        set.browsers.insert(
            browser::HEADLESS.into(),
            BrowserRules {
                options: Some(crate::services::domain::capability::PilotOptions {
                    allow: vec!["interact".into()],
                    ..Default::default()
                }),
                secret_names: vec![],
            },
        );
        assert!(!set.requires_approval(&click, &next));
        let submit = serde_json::json!({"element": "Submit button"});
        assert!(set.requires_approval(&click, &submit));

        // A hand-over always waits for the person, whatever skill.md allows
        let everything = crate::services::domain::capability::PilotOptions {
            allow: crate::services::domain::capability::PILOT_ALLOWS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            ..Default::default()
        };
        set.browsers.insert(
            browser::BROWSER.into(),
            BrowserRules {
                options: Some(everything),
                secret_names: vec![],
            },
        );
        let hand_over = tool_name(browser::BROWSER, handover::TOOL);
        assert!(set.requires_approval(&hand_over, &serde_json::json!({"site": "x.com"})));
    }

    #[test]
    fn a_click_a_policy_refuses_is_refused_not_asked_about() {
        let policy = crate::services::domain::policy::Policy::parse(
            "social",
            "---
applies_to: ['*']
browser: { refuse: [{ controls: [post], sites: [x.com] }] }
---
",
        )
        .unwrap();
        let refusals: Vec<Refusal> = policy
            .rules()
            .browser
            .as_ref()
            .unwrap()
            .refuse
            .iter()
            .map(|rule| Refusal {
                rule: rule.clone(),
                by: "social".into(),
            })
            .collect();
        let mut set = PilotToolset {
            agent: "poster".into(),
            ..PilotToolset::default()
        };
        set.browsers
            .insert(browser::BROWSER.into(), BrowserRules::default());
        set.set_refusals(refusals);
        let click = tool_name(browser::BROWSER, "browser_click");
        let post = serde_json::json!({"element": "Post button", "target": "e21"});
        // Nothing seen yet: it asks (a submit), and is checked when it runs
        assert!(set.requires_approval(&click, &post));
        browser::remember_page("poster", browser::BROWSER, "https://x.com/compose/post");
        assert!(
            !set.requires_approval(&click, &post),
            "refused when it runs"
        );
        let other = serde_json::json!({"element": "Next link", "target": "e3"});
        assert!(set.requires_approval(&click, &other), "the rest still asks");
    }
}
