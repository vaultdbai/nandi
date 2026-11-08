//! Auto-approve rules: what may skip the queue, set by the user (owner,
//! 2026-11-07). One list for every source.
//!
//! - **Teammate**: their Nandi (`instance_id`). Every task is signed by the
//!   requester's Nandi (`relay::sealing`), so this can't be forged. A
//!   GitHub handle is self-declared, so it is never a rule.
//! - **Email address** and **email domain** (the domain or any subdomain):
//!   only mail whose sender passed DMARC for that domain
//!   (`Authentication-Results`, read by the providers). Without it a rule
//!   for `chase.com` would let anyone through who writes "From: chase.com".
//!
//! Screening always runs first: only a clean item can pass a rule, a
//! suspicious one waits, a blocked one is refused or dismissed. Each rule
//! says which actions pass (start work, add news, run a teammate's request).
//!
//! [`matching`] is pure; [`load`] and [`save`] keep the list in local
//! settings (`intake.autoApprove`).

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::screen::Verdict;
use super::IntakeAction;
use crate::db;
use crate::services::requests::local::RequestSource;

const KEY: &str = "intake.autoApprove";
/// Most rules kept
pub const MAX_RULES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum RuleMatch {
    /// A teammate's Nandi; `name` is for showing it
    #[serde(rename_all = "camelCase")]
    Teammate {
        instance_id: String,
        name: String,
    },
    EmailAddress {
        address: String,
    },
    /// The domain and its subdomains (`chase.com` matches `alerts.chase.com`)
    EmailDomain {
        domain: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct AutoApproveRule {
    pub id: String,
    #[serde(rename = "match")]
    pub matcher: RuleMatch,
    /// What passes: `start`, `update`, `run`
    pub actions: Vec<IntakeAction>,
}

/// Where an item came from, as the rules see it
#[derive(Debug, Clone, Default)]
pub struct Origin<'a> {
    pub source: RequestSource,
    /// The sender's address, for mail
    pub sender_email: Option<&'a str>,
    /// The sender passed DMARC for their domain
    pub mail_verified: bool,
    /// The requester's Nandi, for a teammate's task
    pub instance_id: Option<&'a str>,
}

fn domain_of(address: &str) -> Option<String> {
    address
        .rsplit_once('@')
        .map(|(_, d)| d.trim().to_lowercase())
        .filter(|d| !d.is_empty())
}

fn domain_matches(rule: &str, domain: &str) -> bool {
    let rule = rule.trim().trim_start_matches('@').to_lowercase();
    !rule.is_empty() && (domain == rule || domain.ends_with(&format!(".{rule}")))
}

impl RuleMatch {
    fn matches(&self, origin: &Origin) -> bool {
        match self {
            RuleMatch::Teammate { instance_id, .. } => {
                origin.source == RequestSource::Teammate
                    && origin.instance_id == Some(instance_id.as_str())
            }
            RuleMatch::EmailAddress { address } => {
                origin.source == RequestSource::Email
                    && origin.mail_verified
                    && origin
                        .sender_email
                        .is_some_and(|s| s.trim().eq_ignore_ascii_case(address.trim()))
            }
            RuleMatch::EmailDomain { domain } => {
                origin.source == RequestSource::Email
                    && origin.mail_verified
                    && origin
                        .sender_email
                        .and_then(domain_of)
                        .is_some_and(|d| domain_matches(domain, &d))
            }
        }
    }
}

/// The first rule that lets this clean item through, if any. Pure.
pub fn matching<'a>(
    rules: &'a [AutoApproveRule],
    origin: &Origin,
    action: IntakeAction,
    verdict: Verdict,
) -> Option<&'a AutoApproveRule> {
    if verdict != Verdict::Clean {
        return None;
    }
    rules
        .iter()
        .find(|r| r.actions.contains(&action) && r.matcher.matches(origin))
}

/// What the thread says about the rule. Pure.
pub fn describe(rule: &AutoApproveRule) -> String {
    match &rule.matcher {
        RuleMatch::Teammate { name, .. } => format!("requests from {name}"),
        RuleMatch::EmailAddress { address } => format!("mail from {address}"),
        RuleMatch::EmailDomain { domain } => format!("mail from {domain}"),
    }
}

/// Checks a list before it's saved. Pure.
pub fn validate(rules: &[AutoApproveRule]) -> Result<(), String> {
    if rules.len() > MAX_RULES {
        return Err(format!("At most {MAX_RULES} auto-approve rules"));
    }
    for rule in rules {
        if rule.actions.is_empty() {
            return Err("Each rule needs at least one thing it lets through".to_string());
        }
        match &rule.matcher {
            RuleMatch::Teammate { instance_id, .. } if instance_id.trim().is_empty() => {
                return Err("Pick a teammate".to_string())
            }
            RuleMatch::EmailAddress { address }
                if domain_of(address).is_none() || address.contains(char::is_whitespace) =>
            {
                return Err(format!("'{address}' isn't an email address"))
            }
            RuleMatch::EmailDomain { domain } => {
                let d = domain.trim().trim_start_matches('@');
                if !d.contains('.') || d.contains(['@', ' ', '/']) {
                    return Err(format!("'{domain}' isn't a domain like chase.com"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn load(conn: &Connection) -> rusqlite::Result<Vec<AutoApproveRule>> {
    Ok(db::get_setting(conn, KEY)?
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

pub fn save(conn: &Connection, rules: &[AutoApproveRule]) -> rusqlite::Result<()> {
    db::set_setting(
        conn,
        KEY,
        &serde_json::to_value(rules).unwrap_or(serde_json::Value::Null),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(matcher: RuleMatch, actions: &[IntakeAction]) -> AutoApproveRule {
        AutoApproveRule {
            id: "r".to_string(),
            matcher,
            actions: actions.to_vec(),
        }
    }

    fn mail<'a>(from: &'a str, verified: bool) -> Origin<'a> {
        Origin {
            source: RequestSource::Email,
            sender_email: Some(from),
            mail_verified: verified,
            instance_id: None,
        }
    }

    #[test]
    fn a_domain_rule_takes_verified_mail_from_it_and_its_subdomains() {
        let rules = [rule(
            RuleMatch::EmailDomain {
                domain: "chase.com".into(),
            },
            &[IntakeAction::Start],
        )];
        let pass = |o: &Origin| matching(&rules, o, IntakeAction::Start, Verdict::Clean).is_some();
        assert!(pass(&mail("alerts@chase.com", true)));
        assert!(pass(&mail("no-reply@alerts.chase.com", true)));
        // Forged (not DMARC-verified), a lookalike, or another domain: no
        assert!(!pass(&mail("alerts@chase.com", false)));
        assert!(!pass(&mail("alerts@notchase.com", true)));
        assert!(!pass(&mail("alerts@chase.com.evil.io", true)));
    }

    #[test]
    fn screening_and_the_action_still_decide() {
        let rules = [rule(
            RuleMatch::EmailAddress {
                address: "Billing@Acme-Energy.com".into(),
            },
            &[IntakeAction::Start],
        )];
        let origin = mail("billing@acme-energy.com", true);
        assert!(matching(&rules, &origin, IntakeAction::Start, Verdict::Clean).is_some());
        assert!(matching(&rules, &origin, IntakeAction::Start, Verdict::Suspicious).is_none());
        assert!(matching(&rules, &origin, IntakeAction::Update, Verdict::Clean).is_none());
    }

    #[test]
    fn a_teammate_rule_matches_their_nandi_only() {
        let rules = [rule(
            RuleMatch::Teammate {
                instance_id: "inst-lena".into(),
                name: "Lena Park".into(),
            },
            &[IntakeAction::Run],
        )];
        let teammate = |id: &'static str| Origin {
            source: RequestSource::Teammate,
            instance_id: Some(id),
            ..Origin::default()
        };
        assert!(matching(
            &rules,
            &teammate("inst-lena"),
            IntakeAction::Run,
            Verdict::Clean
        )
        .is_some());
        assert!(matching(
            &rules,
            &teammate("inst-sam"),
            IntakeAction::Run,
            Verdict::Clean
        )
        .is_none());
        assert_eq!(describe(&rules[0]), "requests from Lena Park");
    }

    #[test]
    fn rules_are_checked_before_saving() {
        assert!(validate(&[rule(
            RuleMatch::EmailDomain {
                domain: "chase.com".into()
            },
            &[IntakeAction::Start]
        )])
        .is_ok());
        assert!(validate(&[rule(
            RuleMatch::EmailDomain {
                domain: "chase".into()
            },
            &[IntakeAction::Start]
        )])
        .is_err());
        assert!(validate(&[rule(
            RuleMatch::EmailAddress {
                address: "not an address".into()
            },
            &[IntakeAction::Start]
        )])
        .is_err());
        assert!(validate(&[rule(
            RuleMatch::EmailDomain {
                domain: "chase.com".into()
            },
            &[]
        )])
        .is_err());
    }
}
