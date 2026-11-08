//! Access while an agent works: a call to a connector the agent hasn't been
//! granted pauses the run until the person chooses.
//!
//! ```text
//! planned call ─▶ which connectors, read or write? (`need`)
//!   a policy forbids it        ─▶ refused, on the record
//!   granted (or Allow once in this run) ─▶ runs
//!   not yet                    ─▶ access request (an approval, tool `connector_access`)
//!        Allow once            ─▶ this run only
//!        Allow for this agent  ─▶ a grant, saved (default) or on this computer
//!        Don't allow           ─▶ the call fails; not asked again in this run
//! ```
//!
//! A browser pilot opening a site outside the agent's allowed sites asks
//! the same way (family `site`, [`SiteAccess`]): Allow once lets the
//! browser and the egress proxy reach it until the run ends
//! (`pilots::sites`), Always allow writes it into the agent's skill.md
//! (`network.allow`), Don't allow fails the call.
//!
//! Grants are read on every call, so a removed grant stops the next call,
//! even mid-run. The request is an ordinary approval row (it survives
//! restarts, appears in the run, the Inbox and the companion, and expires
//! like any approval). Its `args` are an [`AccessRequest`]; the choice made
//! is written back into them, so the run record says who allowed what.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use crate::db::{self, ApprovalRow};
use crate::model::ApprovalStatus;
use crate::services::agents::SkillConfig;
use crate::services::domain::grant::{ConnectorGrant, GrantAccess, GrantScope};
use crate::services::domain::policy::connector_rule_matches;
use crate::services::grants::{self, GrantInfo, GrantsChanged};
use crate::services::pilots::sites;
use crate::services::repos::agent_locator::{self, Origin};
use crate::services::repos::Repository;
use crate::services::{connectors, email, events, AppState};

/// The synthetic tool of an access request's approval
pub const ACCESS_TOOL: &str = "connector_access";

/// What kind of data a request is about, for the sentence the person reads
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AccessFamily {
    Email,
    Calendar,
    Files,
    /// An installed connector (an MCP server)
    Connector,
    /// A website a browser pilot wants to open ([`SiteAccess`])
    Site,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AccessConnector {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AccessChoice {
    /// This run only
    AllowOnce,
    /// A grant on this computer only
    AllowLocal,
    /// A grant in the repo's settings.yaml
    AllowSaved,
    Deny,
}

/// An access request's approval arguments
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AccessRequest {
    pub agent: String,
    pub family: AccessFamily,
    pub connectors: Vec<AccessConnector>,
    pub access: GrantAccess,
    /// The tool the agent called
    pub tool: String,
    /// What the connector can do with this access, in the publisher's
    /// plain words (`reads` / `writes` of connector.md), when known
    #[serde(default)]
    pub details: Vec<String>,
    /// A policy asks every time: only Allow once and Don't allow
    #[serde(default)]
    pub ask_every_time: bool,
    /// Set once the person chose
    #[serde(default)]
    pub choice: Option<AccessChoice>,
    #[serde(default)]
    pub decided_by: Option<String>,
    /// For family `site`: the site (no connectors)
    #[serde(default)]
    pub site: Option<SiteAccess>,
}

/// A website a browser pilot asked to open
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SiteAccess {
    /// As the browser sends it (punycode for an international name)
    pub host: String,
    /// As people read it (decoded)
    pub display: String,
    /// The pilot that asked (`browser`, `browser-headless`)
    pub pilot: String,
    /// Why the agent wants the page, in its words
    #[serde(default)]
    pub reason: Option<String>,
    /// Whether Always allow can save it: the agent is the user's own, not
    /// a read-only pack agent
    #[serde(default)]
    pub can_save: bool,
    /// Set once the person chose: the grant covers its subdomains too
    #[serde(default)]
    pub subdomains: bool,
}

/// What one call needs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Need {
    pub family: AccessFamily,
    pub connectors: Vec<AccessConnector>,
    pub access: GrantAccess,
    pub details: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Run it, with these connectors (for mail: the mailboxes to read)
    Allowed(Vec<String>),
    /// Pause and ask
    Ask(Box<AccessRequest>),
    /// Refused: the model reads why
    Refused(String),
}

/// The connectors a call reaches and the access it needs; `None` for a
/// call that isn't gated (built-ins, the container's tools)
pub fn need(state: &AppState, tool: &str, args: &Value) -> Option<Need> {
    if email::tool::is_mail_tool(tool) {
        return Some(email::tool::access_need(state, tool, args));
    }
    if let Some(need) = crate::services::workspace::tool::access_need(state, tool, args) {
        return Some(need);
    }
    package_need(state, tool)
}

/// An installed MCP connector's tool (`<id>__<tool>`): read when its
/// connector.md (or `safe_tools`) says so, else write
fn package_need(state: &AppState, tool: &str) -> Option<Need> {
    let manifest = state.store().read(|repo| repo.connectors().clone());
    let declared: Vec<String> = manifest
        .connectors()
        .iter()
        .filter(|c| c.kind == connectors::MCP_KIND)
        .map(|c| c.id.to_string())
        .collect();
    let (id, name) = connectors::parse_prefixed(tool, &declared)?;
    let decl = manifest.get(&id)?;
    let package = state
        .store()
        .repos()
        .connectors
        .load_package(&id)
        .ok()
        .flatten();
    let access = match &package {
        Some(package) => package.tool_access(&name),
        None if connectors::catalog_config_list(decl, "safe_tools").contains(&name) => {
            GrantAccess::Read
        }
        None => GrantAccess::Write,
    };
    let details = package
        .as_ref()
        .map(|p| match access {
            GrantAccess::Read => p.reads.clone(),
            GrantAccess::Write => p.writes.clone(),
        })
        .unwrap_or_default();
    Some(Need {
        family: AccessFamily::Connector,
        connectors: vec![AccessConnector {
            id: id.clone(),
            name: connectors::definition_for(decl).name,
        }],
        access,
        details,
    })
}

/// What the person decided earlier in this run
#[derive(Debug, Default)]
struct RunDecisions {
    /// (connector, access) allowed once
    once: Vec<(String, GrantAccess)>,
    /// (connector, access) refused (or the request expired)
    refused: Vec<(String, GrantAccess)>,
}

async fn run_decisions(state: &AppState, run_id: &str) -> RunDecisions {
    let run = run_id.to_string();
    let rows = state
        .db()
        .call(move |conn| db::run_approvals_for_tool(conn, &run, ACCESS_TOOL))
        .await
        .unwrap_or_default();
    let mut out = RunDecisions::default();
    for row in rows {
        let Ok(request) = serde_json::from_value::<AccessRequest>(row.args.clone()) else {
            continue;
        };
        let pairs = request
            .connectors
            .iter()
            .map(|c| (c.id.clone(), request.access));
        match (row.status, request.choice) {
            (ApprovalStatus::Approved, Some(AccessChoice::AllowOnce)) => out.once.extend(pairs),
            (ApprovalStatus::Rejected | ApprovalStatus::Expired, _) => out.refused.extend(pairs),
            _ => {}
        }
    }
    out
}

/// Policy rules with the policy that set each
type Rules = [(String, String)];

fn policy_hit<'a>(rules: &'a Rules, connector: &str, access: GrantAccess) -> Option<&'a str> {
    rules
        .iter()
        .find(|(rule, _)| connector_rule_matches(rule, connector, access.as_str()))
        .map(|(_, by)| by.as_str())
}

/// Decides one call of `agent` in `run_id`. `None`: not gated
pub async fn decide(
    state: &AppState,
    skill: Option<&SkillConfig>,
    agent: &str,
    run_id: &str,
    tool: &str,
    args: &Value,
) -> Option<Decision> {
    if let Some(decision) = site_decision(state, skill, agent, run_id, tool, args).await {
        return Some(decision);
    }
    let need = need(state, tool, args)?;
    if need.connectors.is_empty() {
        return Some(Decision::Refused(format!(
            "Tool '{tool}' is not available: nothing it needs is connected"
        )));
    }
    let (forbid, ask): (&Rules, &Rules) = match skill {
        Some(skill) => (&skill.connector_forbid, &skill.connector_ask),
        None => (&[], &[]),
    };
    let audit = state.connector_audit();
    let mut open = Vec::new();
    let mut forbidden = Vec::new();
    for connector in &need.connectors {
        match policy_hit(forbid, &connector.id, need.access) {
            Some(by) => {
                let mut entry = connectors::audit::AccessEntry::new(
                    "forbidden",
                    agent,
                    &connector.id,
                    need.access.as_str(),
                );
                entry.by = Some(format!("policy {by}"));
                entry.run_id = Some(run_id.to_string());
                audit.record_access(&entry).await;
                forbidden.push((connector.name.clone(), by.to_string()));
            }
            None => open.push(connector.clone()),
        }
    }
    if open.is_empty() {
        let (name, by) = &forbidden[0];
        return Some(Decision::Refused(format!(
            "Tool '{tool}' is not allowed: the policy '{by}' doesn't let this agent {} with {name}",
            need.access.as_str()
        )));
    }
    let earlier = run_decisions(state, run_id).await;
    let grants = state.grants();
    let mut usable = Vec::new();
    let mut pending = Vec::new();
    let mut every_time = false;
    for connector in open {
        let key = (connector.id.clone(), need.access);
        let always_ask = policy_hit(ask, &connector.id, need.access).is_some();
        every_time |= always_ask;
        let granted = !always_ask && grants.allows(agent, &connector.id, need.access);
        if granted || earlier.once.contains(&key) {
            usable.push(connector.id.clone());
        } else if !earlier.refused.contains(&key) {
            pending.push(connector);
        }
    }
    if !pending.is_empty() {
        return Some(Decision::Ask(Box::new(AccessRequest {
            agent: agent.to_string(),
            family: need.family,
            connectors: pending,
            access: need.access,
            tool: tool.to_string(),
            details: need.details,
            ask_every_time: every_time,
            choice: None,
            decided_by: None,
            site: None,
        })));
    }
    if usable.is_empty() {
        for connector in &need.connectors {
            let mut entry = connectors::audit::AccessEntry::new(
                "refused",
                agent,
                &connector.id,
                need.access.as_str(),
            );
            entry.run_id = Some(run_id.to_string());
            audit.record_access(&entry).await;
        }
        return Some(Decision::Refused(format!(
            "Tool '{tool}' is not allowed: the person didn't let this agent {} with {}. \
             Don't call it again in this task; say what you would have needed.",
            need.access.as_str(),
            need.connectors
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    Some(Decision::Allowed(usable))
}

/// The person's answer to an access request (in the run, the Inbox or the
/// companion): records the choice and any grant, then resolves the
/// approval so the run goes on
pub async fn answer(
    state: &AppState,
    approval_id: &str,
    choice: AccessChoice,
    subdomains: bool,
    who: String,
) -> Result<(), String> {
    let id = approval_id.to_string();
    let approval = state
        .db()
        .call(move |conn| db::get_approval(conn, &id))
        .await?
        .filter(|a| a.tool == ACCESS_TOOL)
        .ok_or("Access request not found")?;
    if approval.status != ApprovalStatus::Pending {
        return Err("This access request is no longer pending".to_string());
    }
    let mut request: AccessRequest = serde_json::from_value(approval.args.clone())
        .map_err(|e| format!("This access request can't be read: {e}"))?;
    if let Some(site) = request.site.take() {
        let site = answer_site(
            state,
            &request.agent,
            &approval.run_id,
            site,
            choice,
            subdomains,
        )?;
        request.site = Some(site);
        return resolve(state, &approval, request, choice, who).await;
    }
    let scope = match choice {
        AccessChoice::AllowLocal => Some(GrantScope::Local),
        AccessChoice::AllowSaved => Some(GrantScope::Saved),
        AccessChoice::AllowOnce | AccessChoice::Deny => None,
    };
    if scope.is_some() && request.ask_every_time {
        return Err("A policy asks every time for this: allow it once, or don't allow it".into());
    }
    let audit = state.connector_audit();
    let mut changed = false;
    for connector in &request.connectors {
        let info = GrantInfo {
            agent: request.agent.clone(),
            connector: connector.id.clone(),
            access: request.access,
            scope: scope.unwrap_or(GrantScope::Local),
        };
        match scope {
            Some(scope) => {
                let grant = ConnectorGrant::new(&request.agent, &connector.id, request.access)
                    .map_err(|e| e.to_string())?;
                changed |= state.grants().grant(grant, scope)?;
                grants::audit(
                    &audit,
                    "granted",
                    &info,
                    Some(who.clone()),
                    Some(approval.run_id.clone()),
                )
                .await;
            }
            None => {
                let event = if choice == AccessChoice::Deny {
                    "refused"
                } else {
                    "allowed_once"
                };
                let mut entry = connectors::audit::AccessEntry::new(
                    event,
                    &request.agent,
                    &connector.id,
                    request.access.as_str(),
                );
                entry.by = Some(who.clone());
                entry.run_id = Some(approval.run_id.clone());
                audit.record_access(&entry).await;
            }
        }
    }
    if changed {
        events::emit(state.bus(), &GrantsChanged {});
    }
    resolve(state, &approval, request, choice, who).await
}

/// Writes the choice into the request and resolves its approval, so the
/// run goes on
async fn resolve(
    state: &AppState,
    approval: &ApprovalRow,
    mut request: AccessRequest,
    choice: AccessChoice,
    who: String,
) -> Result<(), String> {
    request.choice = Some(choice);
    request.decided_by = Some(who);
    let args = serde_json::to_value(&request).map_err(|e| e.to_string())?;
    let row = approval.clone();
    state
        .db()
        .call(move |conn| db::set_approval_args(conn, &row, &args))
        .await?;
    let decision = if choice == AccessChoice::Deny {
        crate::model::ApprovalDecision::Rejected
    } else {
        crate::model::ApprovalDecision::Approved
    };
    crate::services::chat::approve_action(state, None, approval.id.clone(), decision).await
}

/// What the person's choice on a site request does: Allow once lets the
/// browser and the proxy reach it for `run_id`, Always allow writes it into
/// the agent's skill.md. Returns the site as decided.
fn answer_site(
    state: &AppState,
    agent: &str,
    run_id: &str,
    site: SiteAccess,
    choice: AccessChoice,
    subdomains: bool,
) -> Result<SiteAccess, String> {
    let hosts = sites::grant_hosts(&site.host, subdomains);
    match choice {
        AccessChoice::AllowOnce => sites::allow_for_run(agent, run_id, &hosts),
        AccessChoice::AllowSaved if site.can_save => save_sites(state, agent, &hosts)?,
        AccessChoice::AllowSaved => {
            return Err(format!(
                "This agent came with a pack, so its sites can't be changed here: allow {} once, or don't allow it",
                site.display
            ))
        }
        AccessChoice::AllowLocal => {
            return Err("A site is allowed once or always for this agent".to_string())
        }
        AccessChoice::Deny => {}
    }
    log::info!(
        "Site {} for agent '{agent}' in run {run_id}: {choice:?} (subdomains: {subdomains})",
        site.host
    );
    Ok(SiteAccess { subdomains, ..site })
}

/// Adds `hosts` to agent `id`'s allowed sites, through its repository
/// (skill.md is the gold copy)
fn save_sites(state: &AppState, id: &str, hosts: &[String]) -> Result<(), String> {
    let store = state.store();
    let mut agent = store.agent(id)?;
    if agent.allow_sites(hosts).map_err(|e| e.to_string())? {
        store
            .repos()
            .agents
            .save(&agent)
            .map_err(|e| e.to_string())?;
        store.commit(crate::services::domain::EntityKind::Agent, id)?;
    }
    Ok(())
}

/// What the person said earlier in this run about a site
enum SiteEarlier {
    /// Allowed once: the hosts that grant covers
    Once(Vec<String>),
    /// Not allowed, or the request expired
    Refused,
    Unasked,
}

async fn site_earlier(state: &AppState, run_id: &str, host: &str) -> SiteEarlier {
    let run = run_id.to_string();
    let rows = state
        .db()
        .call(move |conn| db::run_approvals_for_tool(conn, &run, ACCESS_TOOL))
        .await
        .unwrap_or_default();
    let mut refused = false;
    for row in rows {
        let Ok(request) = serde_json::from_value::<AccessRequest>(row.args.clone()) else {
            continue;
        };
        let Some(site) = request.site else {
            continue;
        };
        let hosts = sites::grant_hosts(&site.host, site.subdomains);
        if !sites::covers(&hosts, host) {
            continue;
        }
        match (row.status, request.choice) {
            (ApprovalStatus::Approved, Some(AccessChoice::AllowOnce)) => {
                return SiteEarlier::Once(hosts)
            }
            (ApprovalStatus::Rejected | ApprovalStatus::Expired, _) => refused = true,
            _ => {}
        }
    }
    if refused {
        SiteEarlier::Refused
    } else {
        SiteEarlier::Unasked
    }
}

/// A browser pilot call that opens a page outside the agent's allowed
/// sites: ask the person, or say what they said earlier in this run.
/// `None`: not such a call, already allowed, or a site Nandi never asks
/// about (an address, a local name, past a policy's ceiling), which the
/// browser refuses as before.
async fn site_decision(
    state: &AppState,
    skill: Option<&SkillConfig>,
    agent: &str,
    run_id: &str,
    tool: &str,
    args: &Value,
) -> Option<Decision> {
    let (pilot, url) = sites::requested_page(tool, args)?;
    let site = sites::askable(&url)?;
    let caps = &skill?.capabilities;
    if caps.allows_tunnel(&site.host, 443) || caps.allows_request(&site.host, None, "GET") {
        return None;
    }
    match site_earlier(state, run_id, &site.host).await {
        SiteEarlier::Once(hosts) => {
            // Again after a restart, or for the next page on the site
            sites::allow_for_run(agent, run_id, &hosts);
            return None;
        }
        SiteEarlier::Refused => {
            return Some(Decision::Refused(format!(
                "{} isn't allowed: the person didn't let this agent open it. Don't open it again in this task; say what you needed it for.",
                site.display
            )))
        }
        SiteEarlier::Unasked => {}
    }
    if !crate::services::policies::admits_hosts(
        state.store(),
        agent,
        std::slice::from_ref(&site.host),
    ) {
        return None;
    }
    let can_save = agent_locator::locate(state.store().layout(), agent)
        .is_none_or(|place| place.origin == Origin::Local);
    Some(Decision::Ask(Box::new(AccessRequest {
        agent: agent.to_string(),
        family: AccessFamily::Site,
        connectors: Vec::new(),
        access: GrantAccess::Read,
        tool: tool.to_string(),
        details: Vec::new(),
        ask_every_time: false,
        choice: None,
        decided_by: None,
        site: Some(SiteAccess {
            host: site.host,
            display: site.display,
            pilot,
            reason: sites::reason_of(args),
            can_save,
            subdomains: false,
        }),
    })))
}

/// Gives a grant up front (an agent's Connectors panel, Settings)
pub async fn grant_up_front(
    state: &AppState,
    agent: &str,
    connector: &str,
    access: GrantAccess,
    scope: GrantScope,
    who: String,
) -> Result<(), String> {
    state.store().require_agent(agent)?;
    let grant = ConnectorGrant::new(agent, connector, access).map_err(|e| e.to_string())?;
    if state.grants().grant(grant, scope)? {
        let info = GrantInfo {
            agent: agent.to_string(),
            connector: connector.to_string(),
            access,
            scope,
        };
        grants::audit(&state.connector_audit(), "granted", &info, Some(who), None).await;
        events::emit(state.bus(), &GrantsChanged {});
    }
    Ok(())
}

/// Removes a grant; the agent's next call asks again
pub async fn revoke(state: &AppState, grant: GrantInfo, who: String) -> Result<(), String> {
    if state
        .grants()
        .revoke(&grant.agent, &grant.connector, grant.access, grant.scope)?
    {
        grants::audit(&state.connector_audit(), "removed", &grant, Some(who), None).await;
        events::emit(state.bus(), &GrantsChanged {});
    }
    Ok(())
}
