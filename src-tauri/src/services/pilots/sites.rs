//! Sites a person lets a browser pilot open while an agent works.
//!
//! ```text
//! browser_navigate / browser_tabs new ─▶ host in the agent's allowed sites? ─▶ runs
//!   a site Nandi never asks about (an IP, localhost, *.local, a port) ─▶ refused, as before
//!   a policy's network ceiling leaves it out                          ─▶ refused, as before
//!   otherwise ─▶ access request (`services::access`, family `site`)
//!        Allow once        ─▶ this run only ([`allow_for_run`]; the proxy and the
//!                             browser read it until the run ends, [`end_run`])
//!        Always allow      ─▶ the host goes into the agent's skill.md (`network.allow`)
//!        Don't allow       ─▶ the call fails; not asked again in this run
//! ```
//!
//! A grant covers the host only, or the host and its subdomains when the
//! person ticks that.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use serde_json::Value;

use super::browser;
use super::tools::parse_tool_name;
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

/// The `network.allow` hosts a grant for `host` adds
pub fn grant_hosts(host: &str, subdomains: bool) -> Vec<String> {
    let mut hosts = vec![host.to_string()];
    if subdomains {
        hosts.push(format!("*.{host}"));
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

/// Sites allowed once, by agent: the run they're for and its hosts
struct RunSites {
    run_id: String,
    hosts: Vec<String>,
}

static RUN_SITES: LazyLock<RwLock<HashMap<String, RunSites>>> = LazyLock::new(Default::default);

/// Lets `agent`'s browser open `hosts` until `run_id` ends. A grant left
/// from another run of the agent is dropped.
pub fn allow_for_run(agent: &str, run_id: &str, hosts: &[String]) {
    let mut all = RUN_SITES.write().unwrap_or_else(|e| e.into_inner());
    let entry = all.entry(agent.to_string()).or_insert_with(|| RunSites {
        run_id: run_id.to_string(),
        hosts: Vec::new(),
    });
    if entry.run_id != run_id {
        entry.run_id = run_id.to_string();
        entry.hosts.clear();
    }
    for host in hosts {
        if !entry.hosts.contains(host) {
            entry.hosts.push(host.clone());
        }
    }
}

/// Takes back what was allowed once in `run_id` (the run ended)
pub fn end_run(run_id: &str) {
    RUN_SITES
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|_, sites| sites.run_id != run_id);
}

/// `capabilities` plus the sites `agent`'s current run was allowed once
pub fn with_run_sites(agent: &str, mut capabilities: Capabilities) -> Capabilities {
    if let Some(sites) = RUN_SITES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(agent)
    {
        capabilities.network.allow.extend(rules(&sites.hosts));
    }
    capabilities
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
        assert!(!covers(&host_only, "www.example.com"));
        let with_subdomains = grant_hosts("example.com", true);
        assert!(covers(&with_subdomains, "www.example.com"));
        assert!(covers(&with_subdomains, "example.com"));
        assert!(!covers(&with_subdomains, "example.org"));
    }

    #[test]
    fn a_run_s_sites_last_until_it_ends() {
        let caps = || with_run_sites("sites-agent", Capabilities::default());
        assert!(!caps().allows_tunnel("example.com", 443));
        allow_for_run("sites-agent", "run-1", &["example.com".into()]);
        assert!(caps().allows_tunnel("example.com", 443));
        assert!(!with_run_sites("other-agent", Capabilities::default())
            .allows_tunnel("example.com", 443));
        // Another run of the agent starts clean
        allow_for_run("sites-agent", "run-2", &["example.org".into()]);
        assert!(!caps().allows_tunnel("example.com", 443));
        assert!(caps().allows_tunnel("example.org", 443));
        end_run("run-1");
        assert!(caps().allows_tunnel("example.org", 443), "not that run");
        end_run("run-2");
        assert!(!caps().allows_tunnel("example.org", 443));
    }
}
