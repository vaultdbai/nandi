//! Sites a person lets a browser pilot open while an agent works.
//!
//! ```text
//! browser_navigate / browser_tabs new ─▶ host in the agent's allowed sites? ─▶ runs
//!   a site Nandi never asks about (an IP, localhost, *.local, a port) ─▶ refused, as before
//!   a policy's network ceiling leaves it out                          ─▶ refused, as before
//!   otherwise ─▶ access request (`services::access`, family `site`)
//!        Allow once        ─▶ this run only ([`allow_for_run`]; the proxy and the
//!                             browser read it until the run ends, [`end_run`])
//!        Always allow      ─▶ the host goes into the agent's agent.md (`network.allow`)
//!        Don't allow       ─▶ the call fails; not asked again in this run
//!
//! a page on an allowed site needs other hosts ─▶ the proxy refuses them
//!   ([`SiteRefusals`] notes each, by browser identity)
//!   ─▶ the call keeps those the page itself asked for ([`refused_since`],
//!      `browser::call`), names them in its reply and notes them ([`need`])
//!   ─▶ the agent's next page-opening call asks for all of them in one
//!      card ([`take_needed`]): one yes allows them all; a host put on a
//!      card isn't named again for a while ([`was_asked`])
//! ```
//!
//! A grant covers the host only, or the host and its subdomains when the
//! person ticks that. Allow once lasts until the run ends, whatever else
//! the agent runs meanwhile.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, RwLock};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::browser;
use super::tools::parse_tool_name;
use crate::services::domain::lock_box::is_public_suffix;
use crate::services::egress::capabilities::{Capabilities, HostRule};

/// The optional argument in which the agent says why it opens a page
pub const REASON_ARG: &str = "reason";

/// Name endings that are never public websites
const LOCAL_SUFFIXES: &[&str] = &[
    "localhost",
    "local",
    "localdomain",
    "internal",
    "intranet",
    "lan",
    "home.arpa",
];

/// A site a person can be asked about
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskableSite {
    /// As the browser sends it (punycode for an international name)
    pub host: String,
    /// As people read it (decoded); the same as `host` for an ASCII name
    pub display: String,
}

/// The page a browser pilot call opens, with its pilot: `navigate`, or a
/// new tab with a URL
pub fn requested_page(name: &str, args: &Value) -> Option<(String, String)> {
    let (pilot, tool) = parse_tool_name(name)?;
    if !browser::is_browser(pilot) {
        return None;
    }
    browser::url_of(tool, args).map(|url| (pilot.to_string(), url))
}

/// Why the agent opens the page, when it said so
pub fn reason_of(args: &Value) -> Option<String> {
    args.get(REASON_ARG)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(|r| r.chars().take(300).collect())
}

/// The site behind `url` if a person may be asked about it: an http(s)
/// address on its default port whose host is a public domain name. Never
/// an IP address, a single-label or local name, or anything else.
pub fn askable(url: &str) -> Option<AskableSite> {
    let parsed = reqwest::Url::parse(url.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.port().is_some() {
        return None;
    }
    let raw = parsed.host_str()?;
    let bare = raw.trim_start_matches('[').trim_end_matches(']');
    if bare.parse::<std::net::IpAddr>().is_ok() {
        return None;
    }
    let host = raw.trim_end_matches('.').to_ascii_lowercase();
    let labels: Vec<&str> = host.split('.').collect();
    let valid = labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        // A name of digits only is an address in disguise
        && !labels.last().is_some_and(|tld| tld.chars().all(|c| c.is_ascii_digit()));
    let local = LOCAL_SUFFIXES
        .iter()
        .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")));
    if !valid || local {
        return None;
    }
    let (decoded, errors) = idna::domain_to_unicode(&host);
    let display = if errors.is_ok() {
        decoded
    } else {
        host.clone()
    };
    Some(AskableSite { host, display })
}

/// The `network.allow` hosts a grant for `host` adds. `www.example.com`
/// and `example.com` are one site, so a grant covers both, and the
/// wildcard goes on the bare name (`*.example.com`, never
/// `*.www.example.com`). A bare name that is a public suffix
/// (`www.co.uk`) keeps its `www.`
pub fn grant_hosts(host: &str, subdomains: bool) -> Vec<String> {
    let site = host
        .strip_prefix("www.")
        .filter(|bare| bare.contains('.') && !is_public_suffix(bare))
        .unwrap_or(host);
    let mut hosts = vec![site.to_string()];
    if subdomains {
        hosts.push(format!("*.{site}"));
    } else if site != host {
        hosts.push(host.to_string());
    } else if !is_public_suffix(site) {
        hosts.push(format!("www.{site}"));
    }
    hosts
}

/// Whether `hosts` (from [`grant_hosts`]) let a browser open `host`
pub fn covers(hosts: &[String], host: &str) -> bool {
    let caps = Capabilities {
        aws: None,
        network: crate::services::egress::capabilities::NetworkCapabilities {
            allow: rules(hosts),
        },
    };
    caps.allows_tunnel(host, 443)
}

fn rules(hosts: &[String]) -> Vec<HostRule> {
    hosts
        .iter()
        .map(|host| HostRule {
            host: host.clone(),
            methods: Vec::new(),
        })
        .collect()
}

/// Sites allowed once: agent → run → hosts
type RunSites = HashMap<String, HashMap<String, Vec<String>>>;

static RUN_SITES: LazyLock<RwLock<RunSites>> = LazyLock::new(Default::default);

/// Lets `agent`'s browser open `hosts` until `run_id` ends. Another run of
/// the agent allowing sites, or ending, leaves this run's alone.
pub fn allow_for_run(agent: &str, run_id: &str, hosts: &[String]) {
    let mut all = RUN_SITES.write().unwrap_or_else(|e| e.into_inner());
    let allowed = all
        .entry(agent.to_string())
        .or_default()
        .entry(run_id.to_string())
        .or_default();
    for host in hosts {
        if !allowed.contains(host) {
            allowed.push(host.clone());
        }
    }
}

/// Runs that ended leaving their agent's visible browser open for the
/// person (a video, music, a page to read): agent → run ids. Their sites
/// stay allowed until that window closes ([`window_closed`])
static LEFT_OPEN: LazyLock<RwLock<HashMap<String, Vec<String>>>> = LazyLock::new(Default::default);

/// `run_id` ended but left `agent`'s visible browser open: what it allowed
/// once stays with the window, so the page keeps loading (a video's
/// stream) until the window closes
pub fn keep_with_window(agent: &str, run_id: &str) {
    let mut left = LEFT_OPEN.write().unwrap_or_else(|e| e.into_inner());
    let runs = left.entry(agent.to_string()).or_default();
    if !runs.iter().any(|r| r == run_id) {
        runs.push(run_id.to_string());
    }
}

/// `agent`'s visible browser closed: takes back what the runs that left it
/// open allowed once
pub fn window_closed(agent: &str) {
    let runs = LEFT_OPEN
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .remove(agent)
        .unwrap_or_default();
    for run_id in runs {
        end_run(&run_id);
    }
}

/// Takes back what was allowed once in `run_id` (the run ended)
pub fn end_run(run_id: &str) {
    let mut all = RUN_SITES.write().unwrap_or_else(|e| e.into_inner());
    for runs in all.values_mut() {
        runs.remove(run_id);
    }
    all.retain(|_, runs| !runs.is_empty());
}

/// `capabilities` plus the sites `agent`'s running runs were allowed once
/// (its browser is one, whichever run drives it)
pub fn with_run_sites(agent: &str, mut capabilities: Capabilities) -> Capabilities {
    if let Some(runs) = RUN_SITES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(agent)
    {
        for hosts in runs.values() {
            capabilities.network.allow.extend(rules(hosts));
        }
    }
    capabilities
}

// ---------------------------------------------------------------------------
// Hosts a page needed
// ---------------------------------------------------------------------------

/// How long a refusal counts, and how long the hosts a page needed wait
/// for the agent's next page-opening call to ask about them
const NEEDED_FOR: Duration = Duration::from_secs(120);
/// How long a host put on a card isn't named again for that browser
const ASKED_FOR: Duration = Duration::from_secs(600);
/// The most refusals kept per browser
const MOST_REFUSED: usize = 64;
/// The most hosts on one card: it stays readable
const MOST_NEEDED: usize = 12;

/// A host, and when it was refused, needed or asked about
struct Seen {
    host: String,
    at: Instant,
}

/// What Nandi knows about one browser's sites
#[derive(Default)]
struct Browser {
    /// Hosts the proxy refused it, oldest first
    refused: Vec<Seen>,
    /// Hosts its last page asked for and couldn't reach
    needed: Vec<Seen>,
    /// Hosts put on a card lately
    asked: Vec<Seen>,
}

/// Browser identity (`<pilot>:<agent>`) → its sites
static BROWSERS: LazyLock<Mutex<HashMap<String, Browser>>> = LazyLock::new(Default::default);

fn browsers() -> std::sync::MutexGuard<'static, HashMap<String, Browser>> {
    BROWSERS.lock().unwrap_or_else(|e| e.into_inner())
}

fn fresh(list: &mut Vec<Seen>, now: Instant, within: Duration) {
    list.retain(|seen| now.duration_since(seen.at) < within);
}

/// Notes that the proxy refused `identity` a tunnel to `host:port`. Only a
/// site a person may be asked about counts.
pub fn note_refused(identity: &str, host: &str, port: u16) {
    let Some(site) = (port == 443)
        .then(|| askable(&format!("https://{host}/")))
        .flatten()
    else {
        return;
    };
    let now = Instant::now();
    let mut all = browsers();
    let list = &mut all.entry(identity.to_string()).or_default().refused;
    fresh(list, now, NEEDED_FOR);
    list.retain(|seen| seen.host != site.host);
    list.push(Seen {
        host: site.host,
        at: now,
    });
    if list.len() > MOST_REFUSED {
        list.remove(0);
    }
}

/// The hosts refused to `identity` since `since`, oldest first
pub fn refused_since(identity: &str, since: Instant) -> Vec<String> {
    browsers()
        .get(identity)
        .map(|browser| {
            browser
                .refused
                .iter()
                .filter(|seen| seen.at >= since)
                .map(|seen| seen.host.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `host` was put on a card for `identity` lately: it isn't named
/// again, whatever the person answered
pub fn was_asked(identity: &str, host: &str) -> bool {
    let now = Instant::now();
    browsers().get_mut(identity).is_some_and(|browser| {
        fresh(&mut browser.asked, now, ASKED_FOR);
        browser.asked.iter().any(|seen| seen.host == host)
    })
}

/// The hosts `identity`'s page needed and couldn't reach (`browser::call`),
/// for its next page-opening call to ask about. At most [`MOST_NEEDED`].
pub fn need(identity: &str, hosts: &[String]) {
    let now = Instant::now();
    let mut all = browsers();
    let needed = &mut all.entry(identity.to_string()).or_default().needed;
    fresh(needed, now, NEEDED_FOR);
    for host in hosts {
        if !needed.iter().any(|seen| &seen.host == host) {
            needed.push(Seen {
                host: host.clone(),
                at: now,
            });
        }
    }
    while needed.len() > MOST_NEEDED {
        needed.remove(0);
    }
}

/// The hosts `identity`'s pages needed lately, to ask about in one card.
/// They're taken once and remembered as asked.
pub fn take_needed(identity: &str) -> Vec<String> {
    let now = Instant::now();
    let mut all = browsers();
    let Some(browser) = all.get_mut(identity) else {
        return Vec::new();
    };
    fresh(&mut browser.needed, now, NEEDED_FOR);
    let hosts: Vec<String> = browser.needed.drain(..).map(|seen| seen.host).collect();
    for host in &hosts {
        browser.asked.retain(|seen| &seen.host != host);
        browser.asked.push(Seen {
            host: host.clone(),
            at: now,
        });
    }
    hosts
}

/// The egress proxy's refusal hook (`egress::RefusedSite`): notes what an
/// agent's browser was refused. My Nandi browser's window isn't an agent's
/// and is never asked about sites.
pub struct SiteRefusals;

impl crate::services::egress::RefusedSite for SiteRefusals {
    fn refused(&self, identity: &str, host: &str, port: u16) {
        if browser::agent_of_identity(identity).is_some()
            && super::my_browser::kind_of_identity(identity).is_none()
        {
            note_refused(identity, host, port);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn finds_the_page_a_browser_call_opens() {
        let args = json!({"url": "https://example.com/a"});
        assert_eq!(
            requested_page("pilot_browser__browser_navigate", &args),
            Some(("browser".into(), "https://example.com/a".into()))
        );
        assert!(requested_page("pilot_browser-headless__browser_navigate", &args).is_some());
        let tab = json!({"action": "new", "url": "https://example.com"});
        assert!(requested_page("pilot_browser__browser_tabs", &tab).is_some());
        assert!(
            requested_page("pilot_browser__browser_tabs", &json!({"action": "list"})).is_none()
        );
        assert!(requested_page("pilot_browser__browser_click", &args).is_none());
        assert!(requested_page("pilot_opendesk__browser_navigate", &args).is_none());
        assert!(requested_page("shell", &args).is_none());
    }

    #[test]
    fn asks_only_about_public_names_on_default_ports() {
        let site = askable("https://Shop.Example.com./cart").unwrap();
        assert_eq!(site.host, "shop.example.com");
        assert_eq!(site.display, "shop.example.com");
        assert!(askable("http://example.com").is_some());
        for never in [
            "ftp://example.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://example.com:8443/",
            "http://127.0.0.1/",
            "http://10.0.0.5/",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/",
            "http://0x7f000001/",
            "http://localhost/",
            "http://printer.local/",
            "http://metadata.google.internal/",
            "http://router.lan/",
            "http://intranet/",
            "http://1.2.3/",
            "not a url",
        ] {
            assert_eq!(askable(never), None, "{never}");
        }
    }

    #[test]
    fn shows_an_international_name_decoded_and_raw() {
        let site = askable("https://xn--bcher-kva.example/").unwrap();
        assert_eq!(site.host, "xn--bcher-kva.example");
        assert_eq!(site.display, "bücher.example");
        // Typed in Unicode, the browser sends punycode
        assert_eq!(askable("https://bücher.example/").unwrap().host, site.host);
    }

    #[test]
    fn a_grant_covers_the_host_and_subdomains_only_when_asked() {
        let host_only = grant_hosts("example.com", false);
        assert!(covers(&host_only, "example.com"));
        assert!(
            covers(&host_only, "www.example.com"),
            "www is the same site"
        );
        assert!(!covers(&host_only, "shop.example.com"));
        let with_subdomains = grant_hosts("example.com", true);
        assert!(covers(&with_subdomains, "www.example.com"));
        assert!(covers(&with_subdomains, "example.com"));
        assert!(!covers(&with_subdomains, "example.org"));
    }

    /// The wildcard replaces `www.`, so a site without `www.` (or with
    /// another subdomain) is covered too
    #[test]
    fn a_www_grant_is_the_bare_site() {
        assert_eq!(
            grant_hosts("www.youtube.com", true),
            ["youtube.com", "*.youtube.com"]
        );
        let youtube = grant_hosts("www.youtube.com", true);
        assert!(covers(&youtube, "youtube.com"));
        assert!(covers(&youtube, "m.youtube.com"));
        assert!(covers(&youtube, "www.youtube.com"));
        assert_eq!(
            grant_hosts("www.youtube.com", false),
            ["youtube.com", "www.youtube.com"]
        );
        // A bare name that's a public suffix keeps its www.
        assert_eq!(grant_hosts("www.co.uk", true), ["www.co.uk", "*.www.co.uk"]);
        assert_eq!(grant_hosts("co.uk", false), ["co.uk"]);
    }

    /// Allow once lasts the whole run: another run of the agent allowing a
    /// site, or ending, doesn't take it back
    #[test]
    fn an_allow_once_lasts_until_its_run_ends() {
        let caps = || with_run_sites("sites-agent", Capabilities::default());
        assert!(!caps().allows_tunnel("example.com", 443));
        allow_for_run("sites-agent", "run-1", &["example.com".into()]);
        assert!(caps().allows_tunnel("example.com", 443));
        assert!(!with_run_sites("other-agent", Capabilities::default())
            .allows_tunnel("example.com", 443));
        allow_for_run("sites-agent", "run-2", &["example.org".into()]);
        assert!(caps().allows_tunnel("example.com", 443), "run 1 still runs");
        assert!(caps().allows_tunnel("example.org", 443));
        end_run("run-2");
        assert!(caps().allows_tunnel("example.com", 443), "not that run");
        assert!(!caps().allows_tunnel("example.org", 443));
        end_run("run-1");
        assert!(!caps().allows_tunnel("example.com", 443));
    }

    /// A run that leaves the visible browser open keeps its sites until
    /// the window closes, so a video keeps streaming after the run ends
    #[test]
    fn a_window_left_open_keeps_its_sites_until_it_closes() {
        let caps = || with_run_sites("window-agent", Capabilities::default());
        allow_for_run("window-agent", "run-w", &["googlevideo.com".into()]);
        keep_with_window("window-agent", "run-w");
        assert!(
            caps().allows_tunnel("googlevideo.com", 443),
            "the run ended, the window didn't"
        );
        window_closed("other-agent");
        assert!(
            caps().allows_tunnel("googlevideo.com", 443),
            "not its window"
        );
        window_closed("window-agent");
        assert!(!caps().allows_tunnel("googlevideo.com", 443));
    }

    /// What the proxy refused a browser while a page loaded: askable
    /// sites only, each once, the latest refusal last
    #[test]
    fn refused_hosts_are_noted_once() {
        let identity = "browser:refused-agent";
        let before = Instant::now();
        note_refused(identity, "www.gstatic.com", 443);
        note_refused(identity, "encrypted-tbn0.gstatic.com", 443);
        note_refused(identity, "www.gstatic.com", 443);
        note_refused(identity, "127.0.0.1", 443);
        note_refused(identity, "printer.local", 443);
        note_refused(identity, "example.com", 8443);
        note_refused("browser:someone-else", "example.org", 443);
        assert_eq!(
            refused_since(identity, before),
            ["encrypted-tbn0.gstatic.com", "www.gstatic.com"]
        );
        assert!(refused_since(identity, Instant::now()).is_empty());
        assert_eq!(
            refused_since("browser:someone-else", before),
            ["example.org"]
        );
    }

    /// The hosts a page needed are taken once, for one card, at most
    /// [`MOST_NEEDED`]; a host put on a card isn't named again
    #[test]
    fn a_page_s_needed_hosts_are_taken_once_for_one_card() {
        let identity = "browser:needing-agent";
        let hosts = |names: &[&str]| names.iter().map(|h| h.to_string()).collect::<Vec<_>>();
        need(identity, &hosts(&["www.gstatic.com", "fonts.example.org"]));
        need(identity, &hosts(&["www.gstatic.com", "cdn.example.net"]));
        assert!(!was_asked(identity, "www.gstatic.com"));
        assert_eq!(
            take_needed(identity),
            ["www.gstatic.com", "fonts.example.org", "cdn.example.net"]
        );
        assert!(take_needed(identity).is_empty(), "taken once");
        assert!(was_asked(identity, "www.gstatic.com"));
        assert!(!was_asked("browser:other-agent", "www.gstatic.com"));
        let many: Vec<String> = (0..MOST_NEEDED + 3)
            .map(|i| format!("h{i}.example.com"))
            .collect();
        need(identity, &many);
        let kept = take_needed(identity);
        assert_eq!(kept.len(), MOST_NEEDED);
        assert_eq!(kept.last(), many.last());
    }

    /// The proxy's hook notes agents' browsers only: never the person's
    /// window, or a container that isn't a browser
    #[test]
    fn only_agents_browsers_are_noted() {
        use crate::services::egress::RefusedSite;
        let before = Instant::now();
        SiteRefusals.refused("browser-headless:hook-agent", "example.org", 443);
        SiteRefusals.refused("my-browser-chrome:nandi", "example.org", 443);
        SiteRefusals.refused("hook-agent", "example.org", 443);
        assert_eq!(
            refused_since("browser-headless:hook-agent", before),
            ["example.org"]
        );
        assert!(refused_since("my-browser-chrome:nandi", before).is_empty());
        assert!(refused_since("hook-agent", before).is_empty());
    }
}
