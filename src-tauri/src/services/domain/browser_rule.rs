//! Controls a browser pilot may not use: a policy's `browser.refuse` rules.
//!
//! ```yaml
//! browser:
//!   refuse:
//!     - controls: [post, publish, share, send, tweet, reply]
//!       kinds: [submit]          # also Enter, Ctrl+Enter, typing with submit
//!       sites: [linkedin.com, x.com]
//!       except: [start a post]   # opens the composer: not refused
//!       say: Hand over instead, so you press Post yourself.
//!     - controls: [like, follow, connect, message]   # no sites: everywhere
//! ```
//!
//! A rule refuses a browser call when the page is on one of its `sites`
//! (any site when it lists none) and the call either uses a control whose
//! name says one of its `controls` (whole words, any case), or is one of
//! its `kinds` as the browser counts them (`submit`, `pay`, ...), unless the
//! page's own name for the control says one of its `except` phrases. A site `linkedin.com` covers its
//! subdomains too; `host:port` names one port.
//!
//! Pure: what the call and the page say comes from the browser pilot
//! (`pilots::browser`), which also asks the page for the control's own name.

use serde::Deserialize;

use super::capability::{split_port, PILOT_ALLOWS};
use super::{DomainError, DomainResult};

/// `browser:` in a policy
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrowserRules {
    #[serde(default)]
    pub refuse: Vec<ControlRule>,
}

/// One `browser.refuse` item
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRule {
    /// Words on the controls it refuses (`post`, `like`)
    #[serde(default)]
    pub controls: Vec<String>,
    /// Kinds of browser action it refuses ([`PILOT_ALLOWS`] words)
    #[serde(default)]
    pub kinds: Vec<String>,
    /// Where: host names (subdomains included) or `host:port`; empty means
    /// every site
    #[serde(default)]
    pub sites: Vec<String>,
    /// Phrases that keep a control out of the rule (`start a post`)
    #[serde(default)]
    pub except: Vec<String>,
    /// What the agent reads when a call is refused
    #[serde(default)]
    pub say: Option<String>,
}

/// What a refused browser call is checked against
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrowserCall<'a> {
    /// The page's host and port (`None`: the scheme's default)
    pub host: &'a str,
    pub port: Option<u16>,
    /// What names the control: the agent's description of it, its target,
    /// and the page's own name for it
    pub labels: &'a [String],
    /// The page's own name for the control, when the page said. Only this
    /// can match `except`: the agent's own words never exempt a control
    pub name: Option<&'a str>,
    /// The kind the browser counts the call as (`submit`, `interact`, ...)
    pub kind: &'a str,
}

impl BrowserRules {
    pub(super) fn validated(self, policy: &str) -> DomainResult<Self> {
        let refuse = self
            .refuse
            .into_iter()
            .map(|rule| rule.validated(policy))
            .collect::<DomainResult<Vec<_>>>()?;
        Ok(Self { refuse })
    }
}

impl ControlRule {
    fn validated(self, policy: &str) -> DomainResult<Self> {
        let invalid = |reason: String| {
            DomainError::invalid(format!("policy '{policy}'"), "browser.refuse", reason)
        };
        let words = |list: Vec<String>| -> Vec<String> {
            list.iter()
                .map(|w| w.trim().to_lowercase())
                .filter(|w| !w.is_empty())
                .collect()
        };
        let controls = words(self.controls);
        let kinds = words(self.kinds);
        let except = words(self.except);
        let sites: Vec<String> = words(self.sites)
            .into_iter()
            .map(|s| s.trim_start_matches("*.").to_string())
            .collect();
        if controls.is_empty() && kinds.is_empty() {
            return Err(invalid(
                "has an item with neither controls nor kinds, so it refuses nothing".to_string(),
            ));
        }
        if let Some(kind) = kinds.iter().find(|k| !PILOT_ALLOWS.contains(&k.as_str())) {
            return Err(invalid(format!(
                "has the kind '{kind}', which isn't one of: {}",
                PILOT_ALLOWS.join(", ")
            )));
        }
        if let Some(site) = sites.iter().find(|s| !valid_site(s)) {
            return Err(invalid(format!(
                "has the site '{site}'; write a host name (linkedin.com) or host:port"
            )));
        }
        Ok(Self {
            controls,
            kinds,
            sites,
            except,
            say: self
                .say
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
        })
    }

    /// Whether this rule refuses `call`
    pub fn refuses(&self, call: &BrowserCall) -> bool {
        self.covers_site(call.host, call.port)
            && !call.name.is_some_and(|name| says(name, &self.except))
            && (self.kinds.iter().any(|k| k == call.kind)
                || call.labels.iter().any(|label| says(label, &self.controls)))
    }

    /// Whether the rule applies on this page
    pub fn covers_site(&self, host: &str, port: Option<u16>) -> bool {
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        self.sites.is_empty()
            || self.sites.iter().any(|site| {
                let (name, site_port) = split_port(site);
                let same_port =
                    site_port.is_none_or(|p| port.map(|q| q.to_string()) == Some(p.to_string()));
                same_port
                    && (host == name
                        || host
                            .strip_suffix(name)
                            .is_some_and(|rest| rest.ends_with('.')))
            })
    }

    /// One line for the agent page: "post, publish on linkedin.com, x.com"
    pub fn describe(&self) -> String {
        let mut what: Vec<String> = self.controls.clone();
        what.extend(self.kinds.iter().map(|k| format!("any {k}")));
        let what = what.join(", ");
        if self.sites.is_empty() {
            format!("{what} everywhere")
        } else {
            format!("{what} on {}", self.sites.join(", "))
        }
    }
}

fn valid_site(site: &str) -> bool {
    let (name, port) = split_port(site);
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        && port.is_none_or(|p| p.parse::<u16>().is_ok())
}

/// Whether `text` says one of `words`, as whole words in any case
/// ("Posted" isn't "post", "Like" is "like")
pub fn says<S: AsRef<str>>(text: &str, words: &[S]) -> bool {
    let text = text.to_lowercase();
    words.iter().any(|word| {
        let word = word.as_ref().to_lowercase();
        !word.is_empty()
            && text.match_indices(&word).any(|(at, _)| {
                let before = text[..at].chars().next_back();
                let after = text[at + word.len()..].chars().next();
                !before.is_some_and(char::is_alphanumeric)
                    && !after.is_some_and(char::is_alphanumeric)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(yaml: &str) -> DomainResult<BrowserRules> {
        serde_yaml::from_str::<BrowserRules>(yaml)
            .map_err(|e| DomainError::parse("policy 'p'", e))?
            .validated("p")
    }

    fn call<'a>(host: &'a str, labels: &'a [String], kind: &'a str) -> BrowserCall<'a> {
        BrowserCall {
            host,
            port: None,
            labels,
            name: labels.last().map(String::as_str),
            kind,
        }
    }

    fn labels(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn words_match_whole_words_in_any_case() {
        assert!(says("Post", &["post"]));
        assert!(says("button \"Post now\"", &["post"]));
        assert!(!says("Posted 2h ago", &["post"]));
        assert!(!says("Repost", &["post"]));
        assert!(says("Repost", &["repost"]));
        assert!(!says("Start a post", &["like"]));
        assert!(!says("anything", &[""]));
    }

    #[test]
    fn rules_validate_and_normalise() {
        let ok = rules("refuse: [{ controls: [' Post '], sites: ['*.LinkedIn.com', 'x.com:443'], kinds: [Submit] }]")
            .unwrap();
        assert_eq!(ok.refuse[0].controls, ["post"]);
        assert_eq!(ok.refuse[0].sites, ["linkedin.com", "x.com:443"]);
        assert_eq!(ok.refuse[0].kinds, ["submit"]);
        for bad in [
            "refuse: [{ sites: [x.com] }]",
            "refuse: [{ kinds: [teleport] }]",
            "refuse: [{ controls: [post], sites: ['https://x.com/'] }]",
            "refuse: [{ controls: [post], nope: 1 }]",
        ] {
            assert!(rules(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn sites_cover_subdomains_and_ports() {
        let rule =
            &rules("refuse: [{ controls: [post], sites: [linkedin.com, '127.0.0.1:8765'] }]")
                .unwrap()
                .refuse[0];
        assert!(rule.covers_site("linkedin.com", None));
        assert!(rule.covers_site("www.LinkedIn.com", None));
        assert!(!rule.covers_site("notlinkedin.com", None));
        assert!(!rule.covers_site("linkedin.com.evil.test", None));
        assert!(rule.covers_site("127.0.0.1", Some(8765)));
        assert!(!rule.covers_site("127.0.0.1", Some(9000)));
        let everywhere = &rules("refuse: [{ controls: [like] }]").unwrap().refuse[0];
        assert!(everywhere.covers_site("example.com", None));
        assert_eq!(everywhere.describe(), "like everywhere");
    }

    #[test]
    fn a_rule_refuses_by_control_name_or_kind_on_its_sites() {
        let rule = &rules("refuse: [{ controls: [post, reply], kinds: [submit], sites: [x.com] }]")
            .unwrap()
            .refuse[0];
        let post = labels(&["Post button", "e21"]);
        assert!(rule.refuses(&call("x.com", &post, "interact")));
        assert!(!rule.refuses(&call("example.com", &post, "interact")));
        let enter = labels(&[]);
        assert!(rule.refuses(&call("x.com", &enter, "submit")), "Ctrl+Enter");
        let typing = labels(&[]);
        assert!(
            !rule.refuses(&call("x.com", &typing, "interact")),
            "typing into the composer: no control, not a submit"
        );
        let other = labels(&["Search"]);
        assert!(!rule.refuses(&call("x.com", &other, "interact")));
        assert_eq!(rule.describe(), "post, reply, any submit on x.com");
    }

    #[test]
    fn except_keeps_a_control_out() {
        let rule = &rules(
            "refuse: [{ controls: [post], kinds: [submit], sites: [linkedin.com], except: [start a post] }]",
        )
        .unwrap()
        .refuse[0];
        let opener = labels(&["Start a post", "button \"Start a post\""]);
        assert!(!rule.refuses(&call("www.linkedin.com", &opener, "submit")));
        let post = labels(&["button \"Post\""]);
        assert!(rule.refuses(&call("www.linkedin.com", &post, "submit")));
        // The agent's own description can't exempt the real Post button
        let sneaky = labels(&["Start a post", "button \"Post\""]);
        assert!(rule.refuses(&call("www.linkedin.com", &sneaky, "submit")));
        let unknown = BrowserCall {
            name: None,
            ..call("www.linkedin.com", &opener, "submit")
        };
        assert!(
            rule.refuses(&unknown),
            "no name from the page: no exception"
        );
    }
}
