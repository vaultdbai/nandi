//! A key in the Lock box, as its rules say it may be used. Pure: no I/O.
//!
//! The rule: **an agent can use a key, but never sees it, and a key works
//! only at the place it belongs to.** The value lives in the OS keychain
//! (`services::lock_box`); this module holds what decides a release:
//!
//! | Rule        | What it allows                                                        |
//! | ----------- | --------------------------------------------------------------------- |
//! | Bound sites | `https` to a host it names exactly, or `*.example.com` (subdomains)   |
//! | Agents      | Only the agents the person picked                                     |
//! | Lock level  | Open, Ask each time (the default) or Locked (an OS unlock for a time) |
//! | Header      | The one request header the proxy puts it in (`authorization`, ...)    |
//!
//! A key with no bound site is never released. A destination is compared
//! as the URL parser leaves it (lowercase, punycode for an international
//! name), so a lookalike never matches by accident: no suffix, prefix or
//! "contains" match, no port other than 443, no trailing dot, no IP.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Longest key name, in characters
pub const MAX_NAME_CHARS: usize = 64;
/// Most sites one key may be bound to
pub const MAX_HOSTS: usize = 20;
/// The unlock time a Locked key gets unless the person picks another
pub const DEFAULT_UNLOCK_MINUTES: u32 = 15;
/// Longest unlock: a working day
pub const MAX_UNLOCK_MINUTES: u32 = 8 * 60;
/// Longest text before the key in its header (`Bearer `)
const MAX_PREFIX_CHARS: usize = 32;

/// Headers the proxy owns, or that would change where or how a request
/// goes: a key never goes in one of them
const RESERVED_HEADERS: &[&str] = &[
    "connection",
    "content-length",
    "content-type",
    "host",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "proxy-connection",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
];

/// What a key is, for the person reading the list
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum KeyKind {
    ApiKey,
    Password,
    Token,
}

impl KeyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ApiKey => "apiKey",
            Self::Password => "password",
            Self::Token => "token",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "apiKey" => Some(Self::ApiKey),
            "password" => Some(Self::Password),
            "token" => Some(Self::Token),
            _ => None,
        }
    }
}

/// How a key is released on its sites
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum LockLevel {
    /// Released on its sites to its agents, without asking
    Open,
    /// An approval card for each release (the default)
    #[default]
    Ask,
    /// Released only after the OS's own unlock, for a chosen time
    Locked,
}

impl LockLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Ask => "ask",
            Self::Locked => "locked",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "open" => Some(Self::Open),
            "ask" => Some(Self::Ask),
            "locked" => Some(Self::Locked),
            _ => None,
        }
    }

    /// For the record and the logs
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Ask => "ask each time",
            Self::Locked => "locked",
        }
    }
}

/// The request header a key goes in: `name: <prefix><key>`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyHeader {
    /// Lowercase, e.g. `authorization` or `x-api-key`
    pub name: String,
    /// Text before the key, e.g. `Bearer ` (may be empty)
    #[serde(default)]
    pub prefix: String,
}

impl KeyHeader {
    /// Checks and normalizes the header a person chose
    pub fn validated(self) -> Result<Self, String> {
        let name = self.name.trim().to_ascii_lowercase();
        if name.is_empty()
            || name.len() > 64
            || !name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!(
                "'{name}' isn't a header name: use letters, digits and dashes"
            ));
        }
        if RESERVED_HEADERS.contains(&name.as_str()) {
            return Err(format!("A key can't go in the {name} header"));
        }
        let prefix = self.prefix;
        if prefix.chars().count() > MAX_PREFIX_CHARS
            || !prefix.chars().all(|c| c == ' ' || c.is_ascii_graphic())
        {
            return Err(format!(
                "The text before the key is up to {MAX_PREFIX_CHARS} plain characters, on one line"
            ));
        }
        Ok(Self { name, prefix })
    }
}

/// What a person set for one key. Never its value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyRules {
    pub kind: KeyKind,
    /// The sites it may go to: `api.github.com`, `*.example.com`
    pub hosts: Vec<String>,
    /// The agents that may use it, by ID
    pub agents: Vec<String>,
    #[serde(default)]
    pub lock: LockLevel,
    /// How long an OS unlock lasts, for a Locked key
    #[serde(default = "default_unlock_minutes")]
    pub unlock_minutes: u32,
    /// Where the egress proxy puts it; `None`: the proxy never adds it
    #[serde(default)]
    pub header: Option<KeyHeader>,
}

fn default_unlock_minutes() -> u32 {
    DEFAULT_UNLOCK_MINUTES
}

impl KeyRules {
    /// The rules as kept: hosts normalized and deduplicated, agents
    /// trimmed, the unlock time in range, the header checked
    pub fn validated(self) -> Result<Self, String> {
        let mut hosts: Vec<String> = Vec::new();
        for host in &self.hosts {
            let host = normalize_bound_host(host)?;
            if !hosts.contains(&host) {
                hosts.push(host);
            }
        }
        if hosts.len() > MAX_HOSTS {
            return Err(format!("A key may be bound to at most {MAX_HOSTS} sites"));
        }
        let mut agents: Vec<String> = Vec::new();
        for agent in &self.agents {
            let agent = agent.trim().to_string();
            if agent.is_empty() || agent.len() > 128 {
                return Err("An agent's ID can't be empty".to_string());
            }
            if !agents.contains(&agent) {
                agents.push(agent);
            }
        }
        if !(1..=MAX_UNLOCK_MINUTES).contains(&self.unlock_minutes) {
            return Err(format!(
                "An unlock lasts from 1 to {MAX_UNLOCK_MINUTES} minutes"
            ));
        }
        let header = self.header.map(KeyHeader::validated).transpose()?;
        Ok(Self {
            hosts,
            agents,
            header,
            ..self
        })
    }

    pub fn allows_agent(&self, agent: &str) -> bool {
        self.agents.iter().any(|a| a == agent)
    }

    /// The bound host that covers `host`, if one does
    pub fn bound_host_for(&self, host: &str) -> Option<&str> {
        self.hosts
            .iter()
            .find(|bound| host_matches(bound, host))
            .map(String::as_str)
    }
}

/// A key's name: lowercase letters, digits, `.`, `_` and `-`, starting
/// with a letter or digit. It names the keychain entry, so it is checked
/// before anything is stored.
pub fn validate_name(name: &str) -> Result<String, String> {
    let name = name.trim().to_ascii_lowercase();
    let mut chars = name.chars();
    let starts_well = chars.next().is_some_and(|c| c.is_ascii_alphanumeric());
    if !starts_well
        || name.chars().count() > MAX_NAME_CHARS
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(format!(
            "A key's name is 1 to {MAX_NAME_CHARS} lowercase letters, digits, dots, dashes or underscores"
        ));
    }
    Ok(name)
}

/// One DNS label: letters, digits and inner dashes, up to 63 characters
fn is_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && !label.starts_with('-')
        && !label.ends_with('-')
        && label
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// A public name of two or more labels whose last label isn't all digits
fn is_domain(host: &str) -> bool {
    let labels: Vec<&str> = host.split('.').collect();
    labels.len() >= 2
        && labels.iter().all(|l| is_label(l))
        && !labels
            .last()
            .is_some_and(|tld| tld.chars().all(|c| c.is_ascii_digit()))
}

/// A site a key may be bound to, as kept: lowercase ASCII (an
/// international name in punycode, as a URL carries it), no scheme, port,
/// path or trailing dot. `*.example.com` covers the subdomains of
/// `example.com` (not `example.com` itself); nothing broader is accepted.
pub fn normalize_bound_host(raw: &str) -> Result<String, String> {
    let text = raw.trim();
    let refuse = || {
        format!(
            "'{text}' isn't a site a key can be bound to: write a name like api.github.com, or *.example.com for its subdomains"
        )
    };
    if text.is_empty()
        || text.ends_with('.')
        || text.contains(|c: char| c.is_whitespace() || matches!(c, ':' | '/' | '@' | '?' | '#'))
    {
        return Err(refuse());
    }
    let (wildcard, name) = match text.strip_prefix("*.") {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    if name.contains('*') {
        return Err(refuse());
    }
    let ascii = idna::domain_to_ascii(name).map_err(|_| refuse())?;
    if !is_domain(&ascii) {
        return Err(refuse());
    }
    // Two labels at least, so `*.com` (a whole top-level domain) is refused
    Ok(if wildcard {
        format!("*.{ascii}")
    } else {
        ascii
    })
}

/// Whether the bound host `bound` (as [`normalize_bound_host`] keeps it)
/// covers the destination `host` (as the URL parser leaves it). Exact, or
/// one or more whole labels in front of a `*.` rule's name.
pub fn host_matches(bound: &str, host: &str) -> bool {
    if !is_domain(host) {
        return false;
    }
    match bound.strip_prefix("*.") {
        Some(base) => host
            .strip_suffix(base)
            .and_then(|front| front.strip_suffix('.'))
            .is_some_and(|front| front.split('.').all(is_label)),
        None => bound == host,
    }
}

/// Why a key isn't released to a destination
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// The host the key goes to, and the bound host that covers it
    Bound { host: String, bound: String },
    /// Not `https` on its default port, carries a user name, or isn't a name
    NotHttps,
    /// No bound host covers it (a key with none covers nothing)
    NotBound(String),
}

/// Where a request to `url` stands against `rules`: only `https`, on 443,
/// without credentials in the URL, to a name one bound host covers
pub fn destination(rules: &KeyRules, url: &reqwest::Url) -> Destination {
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Destination::NotHttps;
    }
    // `None` for an IP address
    let Some(host) = url.domain() else {
        return Destination::NotHttps;
    };
    match rules.bound_host_for(host) {
        Some(bound) => Destination::Bound {
            host: host.to_string(),
            bound: bound.to_string(),
        },
        None => Destination::NotBound(host.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(hosts: &[&str]) -> KeyRules {
        KeyRules {
            kind: KeyKind::ApiKey,
            hosts: hosts.iter().map(|h| h.to_string()).collect(),
            agents: vec!["writer".into()],
            lock: LockLevel::Open,
            unlock_minutes: DEFAULT_UNLOCK_MINUTES,
            header: Some(KeyHeader {
                name: "Authorization".into(),
                prefix: "Bearer ".into(),
            }),
        }
        .validated()
        .unwrap()
    }

    /// A URL the parser refuses never reaches a key
    fn to(rules: &KeyRules, url: &str) -> Destination {
        reqwest::Url::parse(url)
            .map(|url| destination(rules, &url))
            .unwrap_or(Destination::NotHttps)
    }

    #[test]
    fn names_are_checked_before_they_name_a_keychain_entry() {
        assert_eq!(validate_name(" GitHub-Token ").unwrap(), "github-token");
        for bad in ["", "-x", "a b", "a/b", "ä", &"a".repeat(65), "nandi:x"] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn bound_hosts_are_kept_exact_and_lowercase() {
        assert_eq!(
            normalize_bound_host("API.GitHub.com").unwrap(),
            "api.github.com"
        );
        assert_eq!(
            normalize_bound_host("*.Example.com").unwrap(),
            "*.example.com"
        );
        // An international name is kept as a URL carries it
        assert_eq!(
            normalize_bound_host("bücher.de").unwrap(),
            "xn--bcher-kva.de"
        );
        for bad in [
            "",
            "*",
            "*.com",
            "com",
            "api.github.com.",
            "api.github.com:443",
            "https://api.github.com",
            "api.github.com/path",
            "user@api.github.com",
            "a.*.example.com",
            "*example.com",
            "127.0.0.1",
            "10.0.0.1",
            "[::1]",
            "-bad.example.com",
            "exa mple.com",
        ] {
            assert!(normalize_bound_host(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn an_exact_host_matches_only_itself() {
        let rules = rules(&["api.github.com"]);
        assert!(matches!(
            to(&rules, "https://api.github.com/user"),
            Destination::Bound { .. }
        ));
        // Case is normalized by the URL parser before matching
        assert!(matches!(
            to(&rules, "https://API.GitHub.COM/user"),
            Destination::Bound { .. }
        ));
        // The default port written out is the same address
        assert!(matches!(
            to(&rules, "https://api.github.com:443/user"),
            Destination::Bound { .. }
        ));
        for lookalike in [
            "https://api.github.com.evil.com/",
            "https://api-github.com/",
            "https://evilapi.github.com/",
            "https://github.com/",
            "https://x.api.github.com/",
            "https://api.github.co/",
            "https://api.github.com./",
            "https://api.github.com%2eevil.com/",
            // Cyrillic "а" in "аpi": punycode, not api.github.com
            "https://аpi.github.com/",
            "https://xn--pi-7ld.github.com/",
        ] {
            assert!(
                !matches!(to(&rules, lookalike), Destination::Bound { .. }),
                "{lookalike}"
            );
        }
    }

    #[test]
    fn only_https_on_its_port_without_credentials() {
        let rules = rules(&["api.github.com"]);
        for url in [
            "http://api.github.com/",
            "https://api.github.com:8443/",
            "https://api.github.com:80/",
            "https://user:pw@api.github.com/",
            "https://user@api.github.com/",
            "https://140.82.112.6/",
            "https://[::1]/",
            "ftp://api.github.com/",
        ] {
            assert_eq!(to(&rules, url), Destination::NotHttps, "{url}");
        }
    }

    #[test]
    fn a_subdomain_rule_covers_whole_labels_only() {
        let rules = rules(&["*.example.com"]);
        for ok in ["https://a.example.com/", "https://a.b.example.com/"] {
            assert!(matches!(to(&rules, ok), Destination::Bound { .. }), "{ok}");
        }
        for bad in [
            "https://example.com/",
            "https://evilexample.com/",
            "https://a.example.com.evil.com/",
            "https://example.com.evil.com/",
            "https://a-example.com/",
        ] {
            assert!(
                !matches!(to(&rules, bad), Destination::Bound { .. }),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_key_with_no_bound_host_goes_nowhere() {
        let rules = rules(&[]);
        assert_eq!(
            to(&rules, "https://api.github.com/"),
            Destination::NotBound("api.github.com".into())
        );
    }

    #[test]
    fn an_international_bound_name_matches_its_punycode_url() {
        let rules = rules(&["bücher.de"]);
        assert!(matches!(
            to(&rules, "https://bücher.de/"),
            Destination::Bound { .. }
        ));
        assert!(!matches!(
            to(&rules, "https://bucher.de/"),
            Destination::Bound { .. }
        ));
    }

    #[test]
    fn headers_that_steer_the_request_are_refused() {
        let header = |name: &str, prefix: &str| {
            KeyHeader {
                name: name.into(),
                prefix: prefix.into(),
            }
            .validated()
        };
        assert_eq!(header("X-Api-Key", "").unwrap().name, "x-api-key");
        assert_eq!(
            header("authorization", "Bearer ").unwrap().prefix,
            "Bearer "
        );
        for bad in [
            "host",
            "Proxy-Authorization",
            "content-length",
            "",
            "a b",
            "a:b",
        ] {
            assert!(header(bad, "").is_err(), "{bad}");
        }
        assert!(header("authorization", "Bearer\r\nX: y").is_err());
    }

    #[test]
    fn rules_are_normalized_and_bounded() {
        let mut raw = rules(&["API.github.com", "api.github.com"]);
        raw.agents = vec![" writer ".into(), "writer".into()];
        let kept = raw.validated().unwrap();
        assert_eq!(kept.hosts, ["api.github.com"]);
        assert_eq!(kept.agents, ["writer"]);
        for minutes in [0, MAX_UNLOCK_MINUTES + 1] {
            let mut bad = kept.clone();
            bad.unlock_minutes = minutes;
            assert!(bad.validated().is_err(), "{minutes}");
        }
        assert!(kept.allows_agent("writer"));
        assert!(!kept.allows_agent("Writer"));
    }
}
