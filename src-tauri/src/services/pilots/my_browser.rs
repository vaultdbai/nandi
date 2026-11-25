//! My Nandi browser's window (Settings › Browser): the person's own
//! browsing, on Nandi's browser profile, through Nandi's egress proxy.
//!
//! ```text
//! Open my Nandi browser ─▶ the profile is the person's (`profiles::claim_for_person`)
//!   ─▶ Playwright MCP in window mode (`browser::open_person_window`), through
//!      the egress proxy as `my-browser-<kind>:nandi`, by its own door
//! every connection ─▶ recorded in egress.log under that identity
//!   a site Nandi's policies keep out (a network ceiling) ─▶ refused
//!   anything else                                        ─▶ goes through
//! window closed ─▶ its server and door stop, its token is revoked, the
//!   profile is free for agents
//! ```
//!
//! Nothing here asks "Allow this site?" or writes into Nandi's agent.md:
//! the person's browsing never widens the sites the Nandi agent may open.
//! Addresses, local names and the cloud metadata address are refused as
//! for any browser (`egress::address`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use super::browser;
use super::profiles::{self, BrowserKind, Claim, Holder};
use crate::services::builtin_agents::DEFAULT_AGENT;
use crate::services::egress::capabilities::Capabilities;
use crate::services::repo_store::RepoStore;

/// How often an open window is checked for having closed
const WATCH_EVERY: Duration = Duration::from_secs(2);

/// The window's pilot name in its proxy identity
pub fn pilot_id(kind: BrowserKind) -> &'static str {
    match kind {
        BrowserKind::Chrome => "my-browser-chrome",
        BrowserKind::Edge => "my-browser-edge",
    }
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

/// What a window may reach: anything Nandi's policies don't keep out.
/// Never Nandi's own allowed sites: those are the agent's, not the
/// person's.
pub fn capabilities(store: &RepoStore) -> Result<Capabilities, String> {
    crate::services::policies::network_ceiling(store, DEFAULT_AGENT)
}

/// Open windows: browser type → the session ID of the window open now
static SESSIONS: LazyLock<Mutex<HashMap<BrowserKind, String>>> = LazyLock::new(Default::default);

fn sessions() -> std::sync::MutexGuard<'static, HashMap<BrowserKind, String>> {
    SESSIONS.lock().unwrap_or_else(|e| e.into_inner())
}

/// A new window's session: its ID
fn begin_session(kind: BrowserKind) -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    sessions().insert(kind, id.clone());
    id
}

fn is_session(kind: BrowserKind, id: &str) -> bool {
    sessions().get(&kind).is_some_and(|s| s == id)
}

/// Opens the person's window on `kind`'s shared profile. `Ok(Err(holder))`
/// when someone else has the profile open. Opening it again while it's open
/// adds a tab.
pub async fn open(kind: BrowserKind) -> Result<Result<(), Holder>, String> {
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
            end(kind, id).await;
        }
        return Err(format!("Couldn't open {}: {e}", kind.label()));
    }
    if let Some(id) = session {
        tokio::spawn(watch(kind, dir, id));
    }
    log::info!("Opened My Nandi browser ({})", kind.label());
    Ok(Ok(()))
}

/// Waits for the person to close the window: its server stops, or the
/// browser lets go of the profile's lock
async fn watch(kind: BrowserKind, dir: PathBuf, id: String) {
    let mut seen_lock = false;
    loop {
        tokio::time::sleep(WATCH_EVERY).await;
        if !window_open(kind, &dir, &id, &mut seen_lock).await {
            break;
        }
    }
    end(kind, &id).await;
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
pub async fn close(kind: BrowserKind) {
    let id = sessions().get(&kind).cloned();
    if let Some(id) = id {
        end(kind, &id).await;
    }
}

/// Session `id`'s window closed (or never opened): its server, door and
/// token go, and the profile is free. Nothing when another session has
/// the window now.
async fn end(kind: BrowserKind, id: &str) {
    if !is_session(kind, id) {
        return;
    }
    browser::stop(DEFAULT_AGENT, pilot_id(kind)).await;
    sessions().remove(&kind);
    profiles::release_person(kind);
    log::info!("My Nandi browser ({}) closed", kind.label());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::EntityKind;
    use crate::services::AppState;

    #[test]
    fn the_window_is_nandi_s_browser() {
        assert_eq!(identity(BrowserKind::Chrome), "my-browser-chrome:nandi");
        assert_eq!(identity(BrowserKind::Edge), "my-browser-edge:nandi");
        for kind in BrowserKind::ALL {
            assert_eq!(kind_of_identity(&identity(kind)), Some(kind));
            assert_eq!(
                browser::agent_of_identity(&identity(kind)),
                Some(DEFAULT_AGENT),
                "recorded as Nandi's"
            );
        }
        assert_eq!(kind_of_identity("browser:nandi"), None, "an agent's own");
        assert_eq!(kind_of_identity("my-browser-chrome:writer"), None);
    }

    /// Nandi with one allowed site
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

    fn add_policy(state: &AppState, id: &str, front: &str) {
        let dir = state.store().layout().policies_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.md")),
            format!("---\n{front}\n---\nWhy.\n"),
        )
        .unwrap();
    }

    /// The person's window reaches any public site, not just Nandi's, and
    /// browsing never touches Nandi's agent.md
    #[tokio::test]
    async fn the_window_reaches_any_site_and_leaves_nandi_s_sites_alone() {
        let state = AppState::for_tests(vec![]);
        add_nandi(&state);
        let agent_md = state.store().layout().agent_md_path(DEFAULT_AGENT);
        let before = std::fs::read_to_string(&agent_md).unwrap();
        let caps = capabilities(state.store()).unwrap();
        for host in [
            "example.com",
            "www.google.com",
            "encrypted-tbn0.gstatic.com",
        ] {
            assert!(caps.allows_tunnel(host, 443), "{host}");
        }
        assert_eq!(std::fs::read_to_string(&agent_md).unwrap(), before);
        let nandi = state.store().skill(DEFAULT_AGENT).unwrap().capabilities;
        assert!(
            !nandi.allows_tunnel("www.google.com", 443),
            "the Nandi agent keeps its own sites"
        );
    }

    /// A Nandi policy's network ceiling still blocks the window; a policy
    /// for other agents doesn't
    #[tokio::test]
    async fn a_policy_s_ceiling_blocks_the_window() {
        let state = AppState::for_tests(vec![]);
        add_nandi(&state);
        add_policy(
            &state,
            "work-only",
            &format!(
                "applies_to: [agent:{DEFAULT_AGENT}]\nnetwork: {{ allow: [{{ host: '*.example.com' }}, {{ host: example.com }}] }}"
            ),
        );
        add_policy(
            &state,
            "others",
            "applies_to: [agent:writer]\nnetwork: { allow: [{ host: writer.example }] }",
        );
        let caps = capabilities(state.store()).unwrap();
        assert!(caps.allows_tunnel("example.com", 443));
        assert!(caps.allows_tunnel("docs.example.com", 443));
        assert!(
            !caps.allows_tunnel("www.google.com", 443),
            "past the ceiling"
        );
        assert!(!caps.allows_tunnel("writer.example", 443));
    }
}
