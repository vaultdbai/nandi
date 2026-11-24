//! My Nandi browser's window (Settings › Browser) is the Nandi agent's
//! browser, driven by the person instead of a model.
//!
//! ```text
//! Open my Nandi browser ─▶ the profile is the person's (`profiles::claim_for_person`)
//!   ─▶ Playwright MCP in window mode (`browser::open_person_window`), through
//!      the egress proxy as `my-browser-<kind>:nandi`, with Nandi's allowed sites
//! a site outside them ─▶ the proxy refuses the tunnel and tells [`Asker`]
//!   ─▶ "Allow this site?" (an access request, family `site`, in a Nandi run
//!      of its own in the conversation `browser:<kind>`)
//!        Allow once   ─▶ until the window closes
//!        Always allow ─▶ into Nandi's agent.md (`network.allow`)
//!        Don't allow  ─▶ not asked again while the window is open
//!   then the person reloads the page
//! window closed ─▶ its server stops, its token is revoked, the profile is
//!   free for agents, a request still open expires
//! ```
//!
//! A site Nandi never asks about (an address, a local name, a port, past a
//! policy's ceiling) is refused as it is for any agent.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use super::browser;
use super::profiles::{self, BrowserKind, Claim, Holder};
use super::sites::{self, AskableSite};
use crate::db::{self, ApprovalRow, MessageRow, PlannedCall};
use crate::model::{ApprovalStatus, RunStatus, RunSummary, SourceType};
use crate::services::access::{AccessFamily, AccessRequest, SiteAccess, ACCESS_TOOL};
use crate::services::builtin_agents::DEFAULT_AGENT;
use crate::services::domain::grant::GrantAccess;
use crate::services::egress::capabilities::Capabilities;
use crate::services::events::RunContext;
use crate::services::repo_store::RepoStore;
use crate::services::repos::agent_locator::{self, Origin};
use crate::services::AppState;

/// `approvals.tool_call_id` prefix of a site request from the window
const CALL_PREFIX: &str = "mybrowser:";
/// How often an open window is checked for having closed
const WATCH_EVERY: Duration = Duration::from_secs(2);

/// The window's pilot name in its proxy identity and its site requests
pub fn pilot_id(kind: BrowserKind) -> &'static str {
    match kind {
        BrowserKind::Chrome => "my-browser-chrome",
        BrowserKind::Edge => "my-browser-edge",
    }
}

/// The browser type behind a window's pilot name
pub fn kind_of_pilot(pilot: &str) -> Option<BrowserKind> {
    BrowserKind::ALL.into_iter().find(|k| pilot_id(*k) == pilot)
}

/// The egress proxy's name for `kind`'s window: Nandi's
pub fn identity(kind: BrowserKind) -> String {
    browser::identity(pilot_id(kind), DEFAULT_AGENT)
}

/// The window behind a proxy identity, if it is one
pub fn kind_of_identity(identity: &str) -> Option<BrowserKind> {
    BrowserKind::ALL
        .into_iter()
        .find(|k| self::identity(*k) == identity)
}

/// What `kind`'s window may reach: Nandi's allowed sites, plus those the
/// person allowed once while it's open
pub fn capabilities(store: &RepoStore, kind: BrowserKind) -> Result<Capabilities, String> {
    store
        .skill(DEFAULT_AGENT)
        .map(|skill| sites::with_run_sites(&identity(kind), skill.capabilities))
}

/// The site behind a refused tunnel to `host:port`, if the person may be
/// asked about it (https on its default port, a public name)
fn askable_at(host: &str, port: u16) -> Option<AskableSite> {
    (port == 443)
        .then(|| sites::askable(&format!("https://{host}/")))
        .flatten()
}

/// One open window
struct Session {
    /// The key its sites allowed once live under (`sites::allow_for_run`)
    id: String,
    /// Hosts asked about while it's open: each is asked once
    asked: HashSet<String>,
    /// The Nandi run its open site requests are in
    run: Option<RunContext>,
}

static SESSIONS: LazyLock<Mutex<HashMap<BrowserKind, Session>>> = LazyLock::new(Default::default);
/// Asking, answering and closing one at a time, so a window has one run
/// for its open requests
static ASKING: LazyLock<tokio::sync::Mutex<()>> = LazyLock::new(Default::default);

fn sessions() -> std::sync::MutexGuard<'static, HashMap<BrowserKind, Session>> {
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner())
}

/// A new window's session: its ID
fn begin_session(kind: BrowserKind) -> String {
    let id = format!("{}:{}", identity(kind), uuid::Uuid::new_v4().simple());
    sessions().insert(
        kind,
        Session {
            id: id.clone(),
            asked: HashSet::new(),
            run: None,
        },
    );
    id
}

fn is_session(kind: BrowserKind, id: &str) -> bool {
    sessions().get(&kind).is_some_and(|s| s.id == id)
}

/// Opens the person's window on `kind`'s shared profile. `Ok(Err(holder))`
/// when someone else has the profile open. Opening it again while it's open
/// adds a tab.
pub async fn open(state: &AppState, kind: BrowserKind) -> Result<Result<(), Holder>, String> {
    if kind.executable().is_none() {
        return Err(format!("{} isn't on this computer", kind.label()));
    }
    let local = crate::utils::nandi_config::local_dir()?;
    let dir = profiles::create(&local, kind)?;
    let claim = match profiles::claim_for_person(kind, &dir) {
        Ok(claim) => claim,
        Err(holder) => return Ok(Err(holder)),
    };
    let session = (claim == Claim::Free).then(|| begin_session(kind));
    if let Err(e) = browser::open_person_window(&local, kind, DEFAULT_AGENT, pilot_id(kind)).await {
        if let Some(id) = &session {
            end(state, kind, id).await;
        }
        return Err(format!("Couldn't open {}: {e}", kind.label()));
    }
    if let Some(id) = session {
        tokio::spawn(watch(state.clone(), kind, dir, id));
    }
    log::info!("Opened My Nandi browser ({}) as Nandi", kind.label());
    Ok(Ok(()))
}

/// Waits for the person to close the window: its server stops, or the
/// browser lets go of the profile's lock
async fn watch(state: AppState, kind: BrowserKind, dir: PathBuf, id: String) {
    let mut seen_lock = false;
    loop {
        tokio::time::sleep(WATCH_EVERY).await;
        if !window_open(kind, &dir, &id, &mut seen_lock).await {
            break;
        }
    }
    end(&state, kind, &id).await;
}

async fn window_open(kind: BrowserKind, dir: &Path, id: &str, seen_lock: &mut bool) -> bool {
    if !is_session(kind, id) {
        return false;
    }
    let locked = profiles::locked_on_disk(dir);
    *seen_lock |= locked;
    browser::person_window_runs(DEFAULT_AGENT, pilot_id(kind)).await && (locked || !*seen_lock)
}

/// Closes `kind`'s window, if it's open (before a sign-out wipes it)
pub async fn close(state: &AppState, kind: BrowserKind) {
    let id = sessions().get(&kind).map(|s| s.id.clone());
    if let Some(id) = id {
        end(state, kind, &id).await;
    }
}

/// Session `id`'s window closed (or never opened): its server and token
/// go, its sites allowed once end, a request still open expires, the
/// profile is free. Nothing when another session has the window now.
async fn end(state: &AppState, kind: BrowserKind, id: &str) {
    let _asking = ASKING.lock().await;
    if !is_session(kind, id) {
        return;
    }
    browser::stop(DEFAULT_AGENT, pilot_id(kind)).await;
    let session = sessions().remove(&kind);
    if let Some(session) = session {
        sites::end_run(&session.id);
        if let Some(run) = session.run {
            if let Err(e) = state.bus().run_done(&run, RunStatus::Completed, None).await {
                log::warn!("Couldn't end My Nandi browser's run: {e}");
            }
        }
    }
    profiles::release_person(kind);
    log::info!("My Nandi browser ({}) closed", kind.label());
}

/// Tells the window's person about a site the proxy refused it
pub struct Asker(pub AppState);

impl crate::services::egress::RefusedSite for Asker {
    fn refused(&self, identity: &str, host: &str, port: u16) {
        let Some(kind) = kind_of_identity(identity) else {
            return;
        };
        let Some(site) = askable_at(host, port) else {
            return;
        };
        let state = self.0.clone();
        state.runtime().clone().spawn(async move {
            if let Err(e) = ask(&state, kind, site).await {
                log::warn!("Couldn't ask about a site in My Nandi browser: {e}");
            }
        });
    }
}

/// "Allow this site?" for `site`, opened in `kind`'s window: once per site
/// while it's open, never for one Nandi may open already or a policy keeps
/// out
async fn ask(state: &AppState, kind: BrowserKind, site: AskableSite) -> Result<(), String> {
    let _asking = ASKING.lock().await;
    let run = {
        let mut sessions = sessions();
        let Some(session) = sessions.get_mut(&kind) else {
            return Ok(());
        };
        if !session.asked.insert(site.host.clone()) {
            return Ok(());
        }
        session.run.clone()
    };
    if capabilities(state.store(), kind)?.allows_tunnel(&site.host, 443) {
        return Ok(());
    }
    if !crate::services::policies::admits_hosts(
        state.store(),
        DEFAULT_AGENT,
        std::slice::from_ref(&site.host),
    ) {
        return Ok(());
    }
    let ctx = match run {
        Some(ctx) => ctx,
        None => {
            let ctx = start_run(state, kind).await?;
            if let Some(session) = sessions().get_mut(&kind) {
                session.run = Some(ctx.clone());
            }
            ctx
        }
    };
    let can_save = agent_locator::locate(state.store().layout(), DEFAULT_AGENT)
        .is_none_or(|place| place.origin == Origin::Local);
    let request = AccessRequest {
        agent: DEFAULT_AGENT.to_string(),
        family: AccessFamily::Site,
        connectors: Vec::new(),
        access: GrantAccess::Read,
        tool: format!("pilot_{}__browser_navigate", pilot_id(kind)),
        details: Vec::new(),
        ask_every_time: false,
        choice: None,
        decided_by: None,
        site: Some(SiteAccess {
            host: site.host,
            display: site.display,
            pilot: pilot_id(kind).to_string(),
            reason: Some(format!(
                "You opened it in My Nandi browser ({})",
                kind.label()
            )),
            can_save,
            subdomains: false,
        }),
        mailboxes: Vec::new(),
        granted_mailboxes: None,
    };
    let call = PlannedCall {
        id: format!("{CALL_PREFIX}{}", uuid::Uuid::new_v4().simple()),
        tool: ACCESS_TOOL.to_string(),
        args: serde_json::to_value(&request).map_err(|e| e.to_string())?,
        requires_approval: true,
    };
    state.bus().approval_request(&ctx, &call).await?;
    Ok(())
}

/// A Nandi run for the window's site requests, in its own conversation
async fn start_run(state: &AppState, kind: BrowserKind) -> Result<RunContext, String> {
    let title = format!("My Nandi browser ({})", kind.label());
    let conversation = {
        let (browser, title) = (kind.id(), title.clone());
        state
            .db()
            .call(move |conn| db::ensure_browser_conversation(conn, DEFAULT_AGENT, browser, &title))
            .await?
    };
    let run_id = uuid::Uuid::new_v4().to_string();
    let ctx = RunContext {
        agent_id: DEFAULT_AGENT.to_string(),
        agent_name: state.store().display_name(DEFAULT_AGENT),
        conversation_id: conversation.clone(),
        run_id: run_id.clone(),
    };
    let run = RunSummary {
        id: run_id.clone(),
        agent_id: DEFAULT_AGENT.to_string(),
        conversation_id: conversation.clone(),
        status: RunStatus::Running,
        input: title.clone(),
        summary: None,
        started_at: db::ts(db::now_ms()),
        ended_at: None,
        input_tokens: 0,
        output_tokens: 0,
        cost_usd: None,
        model: None,
        source_type: SourceType::User,
        source_id: None,
        source_label: Some(title.clone()),
        priority: crate::services::orchestrator::PRIORITY_USER,
        queued_at: None,
        parent_run_id: None,
        compaction_summary: None,
    };
    let message = MessageRow::new(
        format!("user-{run_id}"),
        &conversation,
        Some(&run_id),
        db::role::USER,
        title,
        db::UserMeta::default(),
    );
    if !state.bus().start_run(ctx.clone(), run, message).await? {
        return Err("My Nandi browser's conversation has a run going already".to_string());
    }
    Ok(ctx)
}

/// Whether `approval` is a site request from the window
pub fn is_window_approval(approval: &ApprovalRow) -> bool {
    approval.tool_call_id.starts_with(CALL_PREFIX)
}

/// Allow once for a request from window `pilot`: `hosts` until the window
/// closes. `false`: `pilot` isn't a window's
pub fn allow_once(pilot: &str, hosts: &[String]) -> bool {
    let Some(kind) = kind_of_pilot(pilot) else {
        return false;
    };
    if let Some(session) = sessions().get(&kind) {
        sites::allow_for_run(&identity(kind), &session.id, hosts);
    }
    true
}

/// Closes a window's site request as decided (or expired). Its run ends
/// once nothing in it waits.
pub async fn close_approval(
    state: &AppState,
    approval: &ApprovalRow,
    status: ApprovalStatus,
) -> Result<(), String> {
    let _asking = ASKING.lock().await;
    let found = sessions().iter().find_map(|(kind, session)| {
        session
            .run
            .clone()
            .filter(|run| run.run_id == approval.run_id)
            .map(|run| (*kind, run))
    });
    let Some((kind, ctx)) = found else {
        return Err("My Nandi browser closed, so this request ended".to_string());
    };
    state
        .bus()
        .approval_closed(&ctx, approval.clone(), status)
        .await?;
    let run_id = ctx.run_id.clone();
    let waiting = state
        .db()
        .call(move |conn| db::pending_approvals_for_run(conn, &run_id))
        .await?;
    if waiting.is_empty() {
        state
            .bus()
            .run_done(&ctx, RunStatus::Completed, None)
            .await?;
        if let Some(session) = sessions().get_mut(&kind) {
            session.run = None;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::access::{self, AccessChoice};
    use crate::services::domain::EntityKind;

    #[test]
    fn the_window_is_nandi_s_browser() {
        assert_eq!(identity(BrowserKind::Chrome), "my-browser-chrome:nandi");
        assert_eq!(identity(BrowserKind::Edge), "my-browser-edge:nandi");
        for kind in BrowserKind::ALL {
            assert_eq!(kind_of_identity(&identity(kind)), Some(kind));
            assert_eq!(kind_of_pilot(pilot_id(kind)), Some(kind));
            assert_eq!(
                browser::agent_of_identity(&identity(kind)),
                Some(DEFAULT_AGENT),
                "the proxy reads Nandi's sites for it"
            );
        }
        assert_eq!(kind_of_identity("browser:nandi"), None, "an agent's own");
        assert_eq!(kind_of_identity("my-browser-chrome:writer"), None);
        assert_eq!(kind_of_pilot(browser::BROWSER), None);
    }

    #[test]
    fn asks_only_about_public_sites_on_https() {
        assert_eq!(
            askable_at("Shop.Example.com", 443).unwrap().host,
            "shop.example.com"
        );
        assert!(askable_at("example.com", 8443).is_none());
        assert!(askable_at("127.0.0.1", 443).is_none());
        assert!(askable_at("printer.local", 443).is_none());
    }

    fn add_nandi(state: &AppState) {
        let path = state.store().layout().agent_md_path(DEFAULT_AGENT);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "---\nname: Nandi\nnetwork:\n  allow:\n    - host: example.com\n---\nHelp.\n",
        )
        .unwrap();
        state
            .store()
            .commit(EntityKind::Agent, DEFAULT_AGENT)
            .unwrap();
    }

    /// Nandi's pending approvals, as the Inbox lists them
    async fn requests(state: &AppState) -> Vec<ApprovalRow> {
        let ids: Vec<String> = state
            .db()
            .call(|conn| db::pending_approvals(conn, Some(DEFAULT_AGENT)))
            .await
            .unwrap()
            .into_iter()
            .map(|pending| pending.id)
            .collect();
        let mut rows = Vec::new();
        for id in ids {
            rows.push(approval(state, &id).await);
        }
        rows
    }

    async fn approval(state: &AppState, id: &str) -> ApprovalRow {
        let id = id.to_string();
        state
            .db()
            .call(move |conn| db::get_approval(conn, &id))
            .await
            .unwrap()
            .unwrap()
    }

    /// A site Nandi isn't allowed, opened in the window: "Allow this site?"
    /// for Nandi, once; Allow once lets the window reach it until it closes
    #[tokio::test]
    async fn a_new_site_in_the_window_asks_allow_this_site() {
        let state = AppState::for_tests(vec![]);
        add_nandi(&state);
        let kind = BrowserKind::Edge;
        let id = begin_session(kind);
        let caps = || capabilities(state.store(), kind).unwrap();
        assert!(
            caps().allows_tunnel("example.com", 443),
            "Nandi's own sites"
        );
        assert!(!caps().allows_tunnel("example.org", 443));

        ask(&state, kind, askable_at("example.org", 443).unwrap())
            .await
            .unwrap();
        ask(&state, kind, askable_at("example.org", 443).unwrap())
            .await
            .unwrap();
        ask(&state, kind, askable_at("example.com", 443).unwrap())
            .await
            .unwrap();
        let pending = requests(&state).await;
        assert_eq!(pending.len(), 1, "asked once, never for an allowed site");
        let card = &pending[0];
        assert!(is_window_approval(card));
        assert_eq!(card.tool, ACCESS_TOOL);
        let request: AccessRequest = serde_json::from_value(card.args.clone()).unwrap();
        assert_eq!(request.agent, DEFAULT_AGENT);
        assert_eq!(request.family, AccessFamily::Site);
        let site = request.site.unwrap();
        assert_eq!(site.host, "example.org");
        assert_eq!(site.pilot, "my-browser-edge");
        let run_id = card.run_id.clone();
        let run = state
            .db()
            .call(move |conn| db::get_run(conn, &run_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.agent_id, DEFAULT_AGENT, "recorded as Nandi's run");
        assert_eq!(run.conversation_id, "browser:edge");

        access::answer(
            &state,
            &card.id,
            AccessChoice::AllowOnce,
            false,
            None,
            "me".into(),
        )
        .await
        .unwrap();
        assert_eq!(
            approval(&state, &card.id).await.status,
            ApprovalStatus::Approved
        );
        assert!(caps().allows_tunnel("example.org", 443), "allowed once");
        let run_id = card.run_id.clone();
        let run = state
            .db()
            .call(move |conn| db::get_run(conn, &run_id))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(run.status, RunStatus::Completed, "nothing waits in it");

        // Another site gets its own request; closing the window ends it
        ask(&state, kind, askable_at("example.net", 443).unwrap())
            .await
            .unwrap();
        let waiting = requests(&state).await;
        assert_eq!(waiting.len(), 1);
        end(&state, kind, &id).await;
        assert_eq!(
            approval(&state, &waiting[0].id).await.status,
            ApprovalStatus::Expired
        );
        assert!(
            !caps().allows_tunnel("example.org", 443),
            "once ends with the window"
        );
        ask(&state, kind, askable_at("example.net", 443).unwrap())
            .await
            .unwrap();
        assert!(
            requests(&state).await.is_empty(),
            "no window, nobody to ask"
        );
    }
}
