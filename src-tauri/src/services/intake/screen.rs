//! Screening what arrives before it can become work: the mechanical
//! phishing and malware signs in the links and the sender's address. Pure
//! and offline: no link is opened and no attachment is downloaded (Nandi
//! never reads attachments), so a malware scanner can be added here later
//! without changing the queue.
//!
//! What the words say (pressure to hurry, a sign-in or payment lure, a
//! borrowed brand) is the Chief of Staff's to read and explain in the
//! item's thread (`thread::prompt_for`), not a word list's.
//!
//! A sign is either a warning or a block. Any block, or
//! [`WARNINGS_TO_BLOCK`] warnings, blocks the item; the user can only
//! dismiss it. Any warning marks it suspicious: the user sees why before
//! approving.

use serde::{Deserialize, Serialize};
use specta::Type;

/// Warnings that add up to a block
const WARNINGS_TO_BLOCK: usize = 3;

/// Link shorteners hide where a link goes
const SHORTENERS: &[&str] = &[
    "bit.ly",
    "tinyurl.com",
    "t.co",
    "goo.gl",
    "is.gd",
    "ow.ly",
    "buff.ly",
    "rebrand.ly",
    "cutt.ly",
    "shorturl.at",
];

/// Files a link should never download
const RISKY_FILES: &[&str] = &[
    ".exe", ".scr", ".js", ".vbs", ".bat", ".cmd", ".ps1", ".msi", ".iso", ".jar", ".apk", ".hta",
    ".lnk",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Clean,
    /// Warnings: the user sees them before approving
    Suspicious,
    /// Can't be approved, only dismissed
    Blocked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Screening {
    pub verdict: Verdict,
    /// Why, in plain words; empty when clean
    pub reasons: Vec<String>,
    /// Every link found
    pub links: Vec<String>,
}

/// What came in, as screened
#[derive(Debug, Clone, Default)]
pub struct Evidence<'a> {
    /// The sender's display name and address, when there is one
    pub sender_name: Option<&'a str>,
    pub sender_email: Option<&'a str>,
    /// Subject, body, and the request the Chief of Staff wrote from them
    pub texts: Vec<&'a str>,
    /// The user's own addresses (lookalikes of their domains are blocked)
    pub own_addresses: Vec<&'a str>,
}

enum Sign {
    Warn(String),
    Block(String),
}

pub fn screen(evidence: &Evidence) -> Screening {
    let text = evidence.texts.join("\n");
    let links = links_in(&text);
    let mut signs = Vec::new();

    for link in &links {
        signs.extend(link_signs(link));
    }
    signs.extend(sender_signs(evidence));

    let blocks: Vec<String> = signs
        .iter()
        .filter_map(|s| match s {
            Sign::Block(r) => Some(r.clone()),
            Sign::Warn(_) => None,
        })
        .collect();
    let warnings: Vec<String> = signs
        .iter()
        .filter_map(|s| match s {
            Sign::Warn(r) => Some(r.clone()),
            Sign::Block(_) => None,
        })
        .collect();
    let verdict = if !blocks.is_empty() || warnings.len() >= WARNINGS_TO_BLOCK {
        Verdict::Blocked
    } else if !warnings.is_empty() {
        Verdict::Suspicious
    } else {
        Verdict::Clean
    };
    Screening {
        verdict,
        reasons: blocks.into_iter().chain(warnings).collect(),
        links,
    }
}

/// `http(s)://` links and bare `www.` ones, trailing punctuation cut
pub fn links_in(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '\'')) {
        let lower = word.to_lowercase();
        let start = ["https://", "http://", "www."]
            .iter()
            .filter_map(|p| lower.find(p))
            .min();
        if let Some(start) = start {
            let link = word[start..].trim_end_matches(|c: char| ".,;:!?)]}".contains(c));
            if link.len() > 4 && !out.iter().any(|l| l == link) {
                out.push(link.to_string());
            }
        }
    }
    out
}

/// The host of a link, lowercased, without port or credentials
pub fn host_of(link: &str) -> String {
    let rest = link.split_once("://").map(|(_, r)| r).unwrap_or(link);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit('@').next().unwrap_or("");
    let host = host.split(':').next().unwrap_or("");
    host.to_lowercase()
}

fn link_signs(link: &str) -> Vec<Sign> {
    let mut signs = Vec::new();
    let host = host_of(link);
    let path = link.to_lowercase();
    if host.parse::<std::net::Ipv4Addr>().is_ok() {
        signs.push(Sign::Block(format!(
            "A link goes to a bare IP address ({host})"
        )));
    }
    if host.split('.').any(|label| label.starts_with("xn--")) {
        signs.push(Sign::Block(format!(
            "A link's address imitates another with lookalike letters ({host})"
        )));
    }
    if link.contains('@') && link.contains("://") {
        let authority = link.split("://").nth(1).unwrap_or("");
        if authority.split('/').next().unwrap_or("").contains('@') {
            signs.push(Sign::Block(format!(
                "A link hides its real address after an @ ({host})"
            )));
        }
    }
    if SHORTENERS.contains(&host.as_str()) {
        signs.push(Sign::Warn(format!(
            "A link is shortened, so where it goes is hidden ({host})"
        )));
    }
    let file = path.split(['?', '#']).next().unwrap_or("");
    if let Some(ext) = RISKY_FILES.iter().find(|e| file.ends_with(*e)) {
        signs.push(Sign::Block(format!(
            "A link downloads a program or script ({ext})"
        )));
    }
    if path.starts_with("http://") {
        signs.push(Sign::Warn(format!("A link isn't secure (http://{host})")));
    }
    signs
}

fn domain_of(address: &str) -> Option<String> {
    address
        .rsplit_once('@')
        .map(|(_, d)| d.trim().trim_end_matches('>').to_lowercase())
        .filter(|d| !d.is_empty())
}

fn sender_signs(evidence: &Evidence) -> Vec<Sign> {
    let mut signs = Vec::new();
    let Some(email) = evidence.sender_email else {
        return signs;
    };
    let Some(domain) = domain_of(email) else {
        return signs;
    };
    if let Some(name) = evidence.sender_name {
        let name_lower = name.to_lowercase();
        // "IT Support <it@company.com>" <evil@x.io>
        if let Some(shown) = name_lower
            .split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '(' | ')'))
            .find(|w| w.contains('@'))
            .and_then(domain_of)
        {
            if shown != domain {
                signs.push(Sign::Warn(format!(
                    "The sender's name shows another address ({shown}) than the one it came from ({domain})"
                )));
            }
        }
    }
    for own in evidence.own_addresses.iter().filter_map(|a| domain_of(a)) {
        if own != domain && is_lookalike(&own, &domain) {
            signs.push(Sign::Block(format!(
                "The sender's domain {domain} imitates yours ({own})"
            )));
        }
    }
    signs
}

/// Free mail: its users hear from every other provider, so nothing near it
/// is a lookalike
const FREE_MAIL: &[&str] = &[
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "yahoo.com",
    "icloud.com",
    "me.com",
    "aol.com",
    "proton.me",
    "protonmail.com",
];

/// One or two letters apart (vau1tdb.ai for vaultdb.ai), or the same name
/// under another ending (vaultdb.co). Never for free mail. Pure, for tests.
fn is_lookalike(own: &str, other: &str) -> bool {
    if FREE_MAIL.contains(&own) || own.len() < 6 {
        return false;
    }
    let name = |d: &str| d.rsplit_once('.').map(|(n, _)| n.to_string());
    (1..=2).contains(&edit_distance(own, other))
        || (name(own).is_some() && name(own) == name(other))
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let cost = usize::from(ca != cb);
            current.push(
                (previous[j] + cost)
                    .min(previous[j + 1] + 1)
                    .min(current[j] + 1),
            );
        }
        previous = current;
    }
    previous[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(texts: &[&'static str]) -> Evidence<'static> {
        Evidence {
            texts: texts.to_vec(),
            ..Evidence::default()
        }
    }

    #[test]
    fn an_ordinary_bill_is_clean() {
        let s = screen(&Evidence {
            sender_name: Some("Acme Energy"),
            sender_email: Some("billing@acme-energy.com"),
            texts: vec![
                "Your November bill",
                "Your bill of $84 is due on Friday. See https://acme-energy.com/account",
            ],
            own_addresses: vec!["dev@vaultdb.ai"],
        });
        assert_eq!(s.verdict, Verdict::Clean, "{:?}", s.reasons);
        assert_eq!(s.links, vec!["https://acme-energy.com/account"]);
    }

    #[test]
    fn what_the_words_say_is_left_to_the_model() {
        // Lures and pressure with nothing mechanical behind them: the
        // Chief of Staff reads them and explains the risk in the thread
        for text in [
            "URGENT: your password will expire today. Log in at https://login-micros0ft.com/reset",
            "Our bank details have changed; please pay by gift card within 24 hours",
        ] {
            let s = screen(&texts(&[text]));
            assert_eq!(s.verdict, Verdict::Clean, "{text}: {:?}", s.reasons);
        }
        let s = screen(&Evidence {
            sender_name: Some("Microsoft Account Team"),
            sender_email: Some("no-reply@mail-secure.io"),
            texts: vec!["Hello"],
            ..Evidence::default()
        });
        assert_eq!(
            s.verdict,
            Verdict::Clean,
            "a borrowed brand: {:?}",
            s.reasons
        );
    }

    #[test]
    fn dangerous_links_are_blocked() {
        for link in [
            "http://192.168.4.20/pay",
            "https://xn--pple-43d.com/id",
            "https://paypal.com@evil.io/login",
            "https://files.example.com/invoice.exe",
        ] {
            let s = screen(&texts(&[link]));
            assert_eq!(s.verdict, Verdict::Blocked, "{link}: {:?}", s.reasons);
        }
    }

    #[test]
    fn a_shortened_link_is_a_warning() {
        let s = screen(&texts(&["Photos from the party: https://bit.ly/3xyz"]));
        assert_eq!(s.verdict, Verdict::Suspicious);
        assert_eq!(s.reasons.len(), 1);
    }

    #[test]
    fn warnings_add_up_to_a_block() {
        let two = screen(&texts(&[
            "Pay via http://pay.example.com or https://bit.ly/3xyz",
        ]));
        assert_eq!(two.reasons.len(), 2, "{:?}", two.reasons);
        assert_eq!(two.verdict, Verdict::Suspicious);
        let three = screen(&texts(&[
            "Pay via http://pay.example.com or https://bit.ly/3xyz or https://tinyurl.com/x",
        ]));
        assert_eq!(three.verdict, Verdict::Blocked, "{:?}", three.reasons);
    }

    #[test]
    fn a_sender_showing_another_address_is_a_warning() {
        let s = screen(&Evidence {
            sender_name: Some("ceo@vaultdb.ai"),
            sender_email: Some("ceo@gmail.com"),
            texts: vec!["Can you do me a favour?"],
            ..Evidence::default()
        });
        assert_eq!(s.verdict, Verdict::Suspicious);
    }

    #[test]
    fn a_lookalike_of_the_users_domain_is_blocked() {
        let s = screen(&Evidence {
            sender_email: Some("ceo@vau1tdb.ai"),
            texts: vec!["Please review"],
            own_addresses: vec!["dev@vaultdb.ai"],
            ..Evidence::default()
        });
        assert_eq!(s.verdict, Verdict::Blocked);
        // A colleague on the same domain is fine
        let s = screen(&Evidence {
            sender_email: Some("lena@vaultdb.ai"),
            texts: vec!["Please review"],
            own_addresses: vec!["dev@vaultdb.ai"],
            ..Evidence::default()
        });
        assert_eq!(s.verdict, Verdict::Clean);
    }

    #[test]
    fn finds_hosts_and_links() {
        assert_eq!(host_of("https://User@Example.com:8443/a?b"), "example.com");
        assert_eq!(
            links_in("see (https://a.com/x), and www.b.org."),
            vec!["https://a.com/x", "www.b.org"]
        );
        assert!(is_lookalike("vaultdb.ai", "vau1tdb.ai"));
        assert!(is_lookalike("vaultdb.ai", "vaultdb.co"));
        assert!(!is_lookalike("gmail.com", "gmail.co"));
        assert!(!is_lookalike("outlook.com", "outlook.de"));
        assert!(!is_lookalike("vaultdb.ai", "acme-energy.com"));
    }
}
