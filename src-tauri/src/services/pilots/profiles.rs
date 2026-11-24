//! My Nandi browser: one shared profile per installed browser type (Chrome,
//! Edge), at `~/.nandi/local/browser/profiles/<chrome|edge>/profile`.
//!
//! - **The person signs in, at their own pace.** Settings › Browser opens a
//!   visible window on the profile as the Nandi agent's browser
//!   ([`super::my_browser`]): the same Playwright launch as an agent's
//!   visible browser, through the egress proxy under Nandi's identity and
//!   Nandi's allowed sites. A new site asks "Allow this site?" first.
//! - **Agents use it only with a grant.** An agent's visible `browser`
//!   pilot uses the shared profile only when the person chose it on the
//!   agent's page (`consent.<agent>.browser.profile`, [`super::store`]).
//!   The agent's browser still goes through the egress proxy, its allowed
//!   sites and per-call approvals, with extensions off, so nothing the
//!   person installed reads an agent's pages. The headless browser never
//!   gets it ([`for_pilot`]).
//! - **Nothing opens it without the proxy.** Every launch on a shared
//!   profile is Playwright MCP's, from `browser::config`; this module starts
//!   no browser itself.
//! - **One browser per profile.** Chrome corrupts a profile two processes
//!   share, so whoever opens it first holds it ([`decide`]); anyone else is
//!   told calmly that it's in use and tries again. Nobody waits: the
//!   person's window may stay open for hours, and a run that waits silently
//!   looks stuck. The profile's own lock file backs this up across restarts
//!   ([`locked_on_disk`]).
//! - **Never the everyday browser.** Nothing here reads or copies the
//!   person's own Chrome or Edge profile, cookies or password store. Saved
//!   passwords come only through the browser's own sign-in and sync inside
//!   this profile. [`profile_dir`] is the only place a shared profile path is
//!   made, and a test checks no code names the everyday profiles.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};
use specta::Type;
use strum::{EnumString, IntoStaticStr};

use super::browser::{BROWSER, BROWSER_DIR, PROFILE_DIR};

/// The folder under `browser/` that holds the shared profiles
pub const PROFILES_DIR: &str = "profiles";

/// A browser type with its own shared profile
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, EnumString, IntoStaticStr,
)]
#[serde(rename_all = "camelCase")]
#[strum(serialize_all = "snake_case")]
pub enum BrowserKind {
    Chrome,
    Edge,
}

impl BrowserKind {
    /// Chrome first: the visible browser's choice when an agent has its own
    /// profile
    pub const ALL: [Self; 2] = [Self::Chrome, Self::Edge];

    /// Its folder name and the grant's value
    pub fn id(self) -> &'static str {
        self.into()
    }

    /// Playwright's name for it
    pub fn channel(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Edge => "msedge",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Chrome => "Chrome",
            Self::Edge => "Edge",
        }
    }

    /// Where its program may be on this computer
    fn candidates(self) -> Vec<PathBuf> {
        let paths: Vec<PathBuf> = if cfg!(windows) {
            let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
            match self {
                Self::Chrome => vec![
                    r"C:\Program Files\Google\Chrome\Application\chrome.exe".into(),
                    r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".into(),
                    Path::new(&local).join(r"Google\Chrome\Application\chrome.exe"),
                ],
                Self::Edge => vec![
                    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
                    r"C:\Program Files\Microsoft\Edge\Application\msedge.exe".into(),
                ],
            }
        } else if cfg!(target_os = "macos") {
            match self {
                Self::Chrome => {
                    vec!["/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into()]
                }
                Self::Edge => {
                    vec!["/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".into()]
                }
            }
        } else {
            match self {
                Self::Chrome => vec![
                    "/usr/bin/google-chrome".into(),
                    "/opt/google/chrome/chrome".into(),
                ],
                Self::Edge => vec!["/usr/bin/microsoft-edge".into()],
            }
        };
        paths
    }

    /// Its program, if it's installed
    pub fn executable(self) -> Option<PathBuf> {
        self.candidates().into_iter().find(|p| p.exists())
    }
}

/// The browser types installed on this computer, Chrome first
pub fn installed() -> Vec<BrowserKind> {
    BrowserKind::ALL
        .into_iter()
        .filter(|k| k.executable().is_some())
        .collect()
}

/// `kind`'s shared profile under `local` (`~/.nandi/local`): never under
/// the repo (the gold copy), never mounted into a container. Its last
/// folder is `profile`, so backups leave it out like every agent's own.
pub fn profile_dir(local: &Path, kind: BrowserKind) -> PathBuf {
    local
        .join(BROWSER_DIR)
        .join(PROFILES_DIR)
        .join(kind.id())
        .join(PROFILE_DIR)
}

/// Whether `kind`'s shared profile has anything in it (sites may remember
/// the person)
pub fn has_profile(local: &Path, kind: BrowserKind) -> bool {
    profile_dir(local, kind)
        .read_dir()
        .is_ok_and(|mut d| d.next().is_some())
}

/// The profile `pilot` uses, given the agent's grant: the shared one only
/// for the visible browser. The headless browser runs in a container and
/// never gets it.
pub fn for_pilot(pilot: &str, granted: Option<BrowserKind>) -> Option<BrowserKind> {
    granted.filter(|_| pilot == BROWSER)
}

/// Creates `kind`'s shared profile folder (only its owner may read it)
pub fn create(local: &Path, kind: BrowserKind) -> Result<PathBuf, String> {
    let dir = profile_dir(local, kind);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Couldn't create {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(dir)
}

// ---------------------------------------------------------------------------
// One browser per profile
// ---------------------------------------------------------------------------

/// Who has a shared profile open
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Holder {
    /// The person's own window (Settings › Browser), or a sign-out wiping it
    Person,
    /// An agent's visible browser, by agent ID
    Agent(String),
}

/// What asking for a shared profile gives
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// Nobody has it: it's yours now
    Free,
    /// You have it already
    Yours,
    /// Someone else has it open
    InUse(Holder),
}

/// Whether `who` may open a profile `holder` has open
pub fn decide(holder: Option<&Holder>, who: &Holder) -> Claim {
    match holder {
        None => Claim::Free,
        Some(h) if h == who => Claim::Yours,
        Some(h) => Claim::InUse(h.clone()),
    }
}

/// The calm answer when `holder` has `kind`'s profile open. `name` turns
/// an agent ID into its name
pub fn in_use_message(kind: BrowserKind, holder: &Holder, name: impl Fn(&str) -> String) -> String {
    let browser = kind.label();
    match holder {
        Holder::Person => format!(
            "My Nandi browser ({browser}) is open in another window right now. Close that window, then try again."
        ),
        Holder::Agent(agent) => format!(
            "My Nandi browser ({browser}) is in use by {} right now. Try again when its run ends.",
            name(agent)
        ),
    }
}

struct Open {
    holder: Holder,
    /// A sign-out is wiping it: only [`end_wipe`] lets it go
    wiping: bool,
}

static OPEN: LazyLock<Mutex<HashMap<BrowserKind, Open>>> = LazyLock::new(Default::default);

fn open_profiles() -> std::sync::MutexGuard<'static, HashMap<BrowserKind, Open>> {
    OPEN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Who has `kind`'s profile open, as far as this app knows
pub fn holder(kind: BrowserKind) -> Option<Holder> {
    open_profiles().get(&kind).map(|o| o.holder.clone())
}

/// Whether a browser has the profile in `dir` open, by Chromium's own lock
/// (a browser this app didn't start, or one from before a restart)
pub fn locked_on_disk(dir: &Path) -> bool {
    #[cfg(windows)]
    {
        // Chromium holds `lockfile` open without sharing while it runs
        let lock = dir.join("lockfile");
        lock.exists() && std::fs::OpenOptions::new().write(true).open(&lock).is_err()
    }
    #[cfg(not(windows))]
    {
        std::fs::symlink_metadata(dir.join("SingletonLock")).is_ok()
    }
}

/// `agent`'s browser asks for `kind`'s profile (in `dir`): `Ok` when it may
/// open it, else who has it
pub fn claim_for_agent(kind: BrowserKind, agent: &str, dir: &Path) -> Result<(), Holder> {
    let who = Holder::Agent(agent.to_string());
    let mut open = open_profiles();
    match decide(open.get(&kind).map(|o| &o.holder), &who) {
        Claim::Yours => Ok(()),
        Claim::InUse(holder) => Err(holder),
        Claim::Free if locked_on_disk(dir) => Err(Holder::Person),
        Claim::Free => {
            open.insert(
                kind,
                Open {
                    holder: who,
                    wiping: false,
                },
            );
            Ok(())
        }
    }
}

/// `agent`'s browser closed: every shared profile it held is free
pub fn release_agent(agent: &str) {
    let gone = Holder::Agent(agent.to_string());
    open_profiles().retain(|_, o| o.holder != gone);
}

/// Playwright's defaults that keep a browser's passwords and cookies in a
/// plain store instead of the system keychain. Every visible browser drops
/// them (`browser::config`), the person's window included, so both read
/// the same sign-ins from the keychain
pub const PLAIN_STORE_ARGS: [&str; 2] = ["--password-store=basic", "--use-mock-keychain"];

/// The person's window asks for `kind`'s profile (in `dir`): `Ok(Free)`
/// when it's theirs now, `Ok(Yours)` when their window has it already,
/// else who has it. A browser this app didn't start counts as the person's.
pub fn claim_for_person(kind: BrowserKind, dir: &Path) -> Result<Claim, Holder> {
    let mut open = open_profiles();
    match decide(open.get(&kind).map(|o| &o.holder), &Holder::Person) {
        Claim::InUse(holder) => Err(holder),
        Claim::Yours if open.get(&kind).is_some_and(|o| o.wiping) => Err(Holder::Person),
        Claim::Yours => Ok(Claim::Yours),
        Claim::Free if locked_on_disk(dir) => Err(Holder::Person),
        Claim::Free => {
            open.insert(
                kind,
                Open {
                    holder: Holder::Person,
                    wiping: false,
                },
            );
            Ok(Claim::Free)
        }
    }
}

/// The person's window on `kind` closed: the profile is free (unless a
/// sign-out is wiping it)
pub fn release_person(kind: BrowserKind) {
    let mut open = open_profiles();
    if open
        .get(&kind)
        .is_some_and(|o| o.holder == Holder::Person && !o.wiping)
    {
        open.remove(&kind);
    }
}

/// Starts wiping `kind`'s profile: keeps agents and a new window out until
/// [`end_wipe`] (the caller closes the person's window). `Err(holder)` when
/// an agent has it open.
pub fn begin_wipe(kind: BrowserKind) -> Result<(), Holder> {
    let mut open = open_profiles();
    if let Some(Open {
        holder: Holder::Agent(agent),
        ..
    }) = open.get(&kind)
    {
        return Err(Holder::Agent(agent.clone()));
    }
    open.insert(
        kind,
        Open {
            holder: Holder::Person,
            wiping: true,
        },
    );
    Ok(())
}

pub fn end_wipe(kind: BrowserKind) {
    let mut open = open_profiles();
    if open.get(&kind).is_some_and(|o| o.wiping) {
        open.remove(&kind);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::pilots::browser::HEADLESS;

    #[test]
    fn shared_profiles_live_under_local_browser_profiles() {
        let home = Path::new("/home/u/.nandi");
        let local = home.join("local");
        assert_eq!(
            profile_dir(&local, BrowserKind::Chrome),
            Path::new("/home/u/.nandi/local/browser/profiles/chrome/profile")
        );
        assert_eq!(
            profile_dir(&local, BrowserKind::Edge),
            Path::new("/home/u/.nandi/local/browser/profiles/edge/profile")
        );
        for kind in BrowserKind::ALL {
            let dir = profile_dir(&local, kind);
            assert!(dir.starts_with(local.join("browser/profiles")));
            assert!(!dir.starts_with(home.join("repo")), "never the gold copy");
        }
    }

    #[test]
    fn the_grant_names_a_browser_type() {
        assert_eq!(BrowserKind::Chrome.id(), "chrome");
        assert_eq!(BrowserKind::Edge.id(), "edge");
        assert_eq!("edge".parse::<BrowserKind>().unwrap(), BrowserKind::Edge);
        assert!("firefox".parse::<BrowserKind>().is_err());
        assert_eq!(BrowserKind::Edge.channel(), "msedge");
        assert_eq!(
            serde_json::to_value(BrowserKind::Chrome).unwrap(),
            serde_json::json!("chrome")
        );
    }

    #[test]
    fn only_the_visible_browser_gets_the_shared_profile() {
        let chrome = Some(BrowserKind::Chrome);
        assert_eq!(for_pilot(BROWSER, chrome), chrome);
        assert_eq!(for_pilot(BROWSER, None), None, "no grant: its own");
        assert_eq!(for_pilot(HEADLESS, chrome), None, "never in a container");
        assert_eq!(for_pilot("opendesk", chrome), None);
    }

    #[test]
    fn whoever_opens_it_first_holds_it() {
        let person = Holder::Person;
        let cos = Holder::Agent("cos".into());
        let shopper = Holder::Agent("shopper".into());
        assert_eq!(decide(None, &cos), Claim::Free);
        assert_eq!(decide(Some(&cos), &cos), Claim::Yours);
        assert_eq!(decide(Some(&cos), &shopper), Claim::InUse(cos.clone()));
        assert_eq!(decide(Some(&person), &cos), Claim::InUse(person.clone()));
        assert_eq!(decide(Some(&cos), &person), Claim::InUse(cos.clone()));
        assert_eq!(decide(Some(&person), &person), Claim::Yours);
    }

    #[test]
    fn in_use_is_said_calmly() {
        let name = |id: &str| format!("'{id}'");
        let by_agent = in_use_message(BrowserKind::Edge, &Holder::Agent("cos".into()), name);
        assert_eq!(
            by_agent,
            "My Nandi browser (Edge) is in use by 'cos' right now. Try again when its run ends."
        );
        let by_person = in_use_message(BrowserKind::Chrome, &Holder::Person, name);
        assert!(by_person.contains("open in another window"), "{by_person}");
    }

    /// Uses a browser type no other test claims, so tests can run together
    #[test]
    fn a_second_agent_is_told_the_profile_is_in_use_until_the_first_lets_go() {
        let dir = tempfile::tempdir().unwrap();
        let kind = BrowserKind::Edge;
        assert_eq!(claim_for_agent(kind, "first", dir.path()), Ok(()));
        assert_eq!(claim_for_agent(kind, "first", dir.path()), Ok(()), "again");
        assert_eq!(
            claim_for_agent(kind, "second", dir.path()),
            Err(Holder::Agent("first".into()))
        );
        assert_eq!(
            begin_wipe(kind),
            Err(Holder::Agent("first".into())),
            "no sign-out under a running agent"
        );
        release_agent("first");
        assert_eq!(holder(kind), None);
        assert_eq!(claim_for_agent(kind, "second", dir.path()), Ok(()));
        release_agent("second");

        // A sign-out keeps agents out until it's done
        assert_eq!(begin_wipe(kind), Ok(()));
        assert_eq!(
            claim_for_agent(kind, "first", dir.path()),
            Err(Holder::Person)
        );
        end_wipe(kind);
        assert_eq!(holder(kind), None);
    }

    #[test]
    fn a_browser_this_app_did_not_start_still_counts() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!locked_on_disk(dir.path()));
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            let lock = dir.path().join("lockfile");
            let held = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .share_mode(0)
                .open(&lock)
                .unwrap();
            assert!(locked_on_disk(dir.path()));
            drop(held);
            assert!(!locked_on_disk(dir.path()), "a stale lock file is free");
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("host-123", dir.path().join("SingletonLock")).unwrap();
            assert!(locked_on_disk(dir.path()));
        }
    }

    /// Uses a browser type no other test claims (the agents' test uses
    /// Edge), so tests can run together
    #[test]
    fn the_person_s_window_holds_the_profile_and_agents_wait() {
        let dir = tempfile::tempdir().unwrap();
        let kind = BrowserKind::Chrome;
        assert_eq!(claim_for_person(kind, dir.path()), Ok(Claim::Free));
        assert_eq!(claim_for_person(kind, dir.path()), Ok(Claim::Yours));
        assert_eq!(
            claim_for_agent(kind, "cos", dir.path()),
            Err(Holder::Person),
            "in use while the person's window is open"
        );
        // A sign-out closes the window; the window's close doesn't free it
        assert_eq!(begin_wipe(kind), Ok(()));
        release_person(kind);
        assert_eq!(
            claim_for_agent(kind, "cos", dir.path()),
            Err(Holder::Person)
        );
        assert_eq!(claim_for_person(kind, dir.path()), Err(Holder::Person));
        end_wipe(kind);
        assert_eq!(holder(kind), None);

        assert_eq!(claim_for_person(kind, dir.path()), Ok(Claim::Free));
        release_person(kind);
        assert_eq!(claim_for_agent(kind, "cos", dir.path()), Ok(()));
        assert_eq!(
            claim_for_person(kind, dir.path()),
            Err(Holder::Agent("cos".into()))
        );
        release_agent("cos");
        assert_eq!(holder(kind), None);
    }

    /// Every launch on a shared profile goes through Playwright MCP and the
    /// egress proxy (`browser::config`): this module, which knows where
    /// Chrome and Edge are, never starts one itself
    #[test]
    fn nothing_here_starts_a_browser_without_the_proxy() {
        let src = include_str!("profiles.rs");
        let code = &src[..src.find("#[cfg(test)]").unwrap()];
        for spawn in ["Command::new", ".spawn()", "std::process"] {
            assert!(!code.contains(spawn), "{spawn}");
        }
    }

    /// The static check: no code names the person's everyday browser
    /// profiles, cookies or password store. Saved passwords reach the
    /// shared profile only through the browser's own sync.
    #[test]
    fn no_code_reads_the_everyday_browser_profile() {
        // Joined here so this file doesn't match itself
        let forbidden: Vec<String> = [
            ["User", " Data"].concat(),
            ["Login", " Data"].concat(),
            ["Local", " State"].concat(),
            ["Application Support/", "Google"].concat(),
            ["Application Support/", "Microsoft Edge"].concat(),
            [".config/", "google-chrome"].concat(),
            [".config/", "microsoft-edge"].concat(),
            ["Chrome Safe", " Storage"].concat(),
        ]
        .into();
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src];
        let mut found = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).unwrap_or_default();
                    for word in &forbidden {
                        if text.contains(word.as_str()) {
                            found.push(format!("{}: {word}", path.display()));
                        }
                    }
                }
            }
        }
        assert!(found.is_empty(), "{found:#?}");
    }
}
