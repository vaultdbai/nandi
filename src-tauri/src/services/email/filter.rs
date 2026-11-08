//! What the Chief of Staff never sees: bulk mail (newsletters,
//! notifications), senders that look like phishing, and mail the user sent
//! only to themselves. Older mail, read only to build the profile, also
//! leaves out what the user never opened and machines' updates. Senders and
//! domains on the watch list always come through.

use super::mail::{Folder, Header};
use super::text::Address;

/// Local parts of machine senders: system updates and notifications
const AUTOMATED: [&str; 22] = [
    "noreply",
    "no-reply",
    "no_reply",
    "donotreply",
    "do-not-reply",
    "do_not_reply",
    "notification",
    "notifications",
    "notify",
    "alert",
    "alerts",
    "mailer-daemon",
    "postmaster",
    "bounce",
    "bounces",
    "automated",
    "system",
    "updates",
    "news",
    "newsletter",
    "digest",
    "marketing",
];

/// Names phishing borrows. One word each, matched as a whole word in the
/// sender's display name
const BRANDS: [&str; 30] = [
    "paypal",
    "microsoft",
    "office365",
    "outlook",
    "apple",
    "icloud",
    "amazon",
    "google",
    "gmail",
    "netflix",
    "docusign",
    "dropbox",
    "dhl",
    "fedex",
    "ups",
    "usps",
    "irs",
    "hmrc",
    "linkedin",
    "facebook",
    "instagram",
    "whatsapp",
    "chase",
    "citibank",
    "hsbc",
    "barclays",
    "wellsfargo",
    "coinbase",
    "binance",
    "metamask",
];

/// A machine sender: `noreply@`, `notifications+abc@`, `alerts-eu@` …
pub fn is_automated(from: &Address) -> bool {
    let local = from.email.split('@').next().unwrap_or_default();
    AUTOMATED.iter().any(|word| {
        local == *word
            || local
                .strip_prefix(word)
                .is_some_and(|rest| rest.starts_with(['+', '-', '_', '.']))
    })
}

/// A sender that looks like phishing: a lookalike (punycode) domain, a
/// display name that is another domain's address, or a display name that
/// claims a brand the sending domain isn't
pub fn looks_suspicious(from: &Address) -> bool {
    let domain = from.domain();
    if domain.split('.').any(|label| label.starts_with("xn--")) {
        return true;
    }
    let Some(name) = from.name.as_deref().map(str::to_ascii_lowercase) else {
        return false;
    };
    let same_domain = |d: &str| d == domain || domain.ends_with(&format!(".{d}"));
    let other_address = name.split_whitespace().any(|word| {
        let word = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        word.rsplit_once('@').is_some_and(|(_, d)| {
            !d.is_empty() && !same_domain(d) && !d.ends_with(&format!(".{domain}"))
        })
    });
    if other_address {
        return true;
    }
    let plain_domain = domain.replace('-', "");
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|word| BRANDS.contains(&word) && !plain_domain.contains(word))
}

/// Headers that mark a message as sent to a list
pub fn is_bulk_by_headers<'a>(headers: impl IntoIterator<Item = (&'a str, &'a str)>) -> bool {
    headers.into_iter().any(|(name, value)| {
        let value = value.trim().to_ascii_lowercase();
        match name.to_ascii_lowercase().as_str() {
            "list-unsubscribe" | "list-id" => !value.is_empty(),
            "precedence" => matches!(value.as_str(), "bulk" | "list" | "junk"),
            "auto-submitted" => !value.is_empty() && value != "no",
            _ => false,
        }
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MailFilter {
    /// The user's own addresses (lowercase)
    me: Vec<String>,
    /// `lena@contoso.com`, `contoso.com` or `@contoso.com` (lowercase)
    watch: Vec<String>,
}

impl MailFilter {
    pub fn new(me: &[String], watch: &[String]) -> Self {
        let clean = |list: &[String]| {
            list.iter()
                .map(|s| s.trim().trim_start_matches('@').to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect()
        };
        Self {
            me: clean(me),
            watch: clean(watch),
        }
    }

    fn is_me(&self, address: &Address) -> bool {
        self.me.contains(&address.email)
    }

    fn watched(&self, address: &Address) -> bool {
        self.watch.iter().any(|entry| {
            if entry.contains('@') {
                address.email == *entry
            } else {
                let domain = address.domain();
                domain == entry || domain.ends_with(&format!(".{entry}"))
            }
        })
    }

    /// Whether the Chief of Staff should see `header`
    pub fn keep(&self, header: &Header) -> bool {
        let sender_watched = header.from.as_ref().is_some_and(|f| self.watched(f));
        if sender_watched {
            return true;
        }
        if header.bulk {
            return false;
        }
        let received = header.folder != Folder::Sent;
        if received && header.from.as_ref().is_some_and(looks_suspicious) {
            return false;
        }
        let from_me = header.from.as_ref().is_some_and(|f| self.is_me(f));
        let recipients: Vec<&Address> = header.to.iter().chain(&header.cc).collect();
        let only_to_me = !recipients.is_empty() && recipients.iter().all(|a| self.is_me(a));
        !(from_me && only_to_me)
    }

    /// Whether older mail is worth reading to build the profile: what
    /// [`Self::keep`] keeps, less mail the user never opened (they didn't
    /// care then) and machines' updates. New mail keeps both.
    pub fn keep_older(&self, header: &Header) -> bool {
        if !self.keep(header) {
            return false;
        }
        if header.from.as_ref().is_some_and(|f| self.watched(f)) {
            return true;
        }
        !header.unread && !header.from.as_ref().is_some_and(is_automated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(from: &str, to: &[&str], bulk: bool) -> Header {
        Header {
            id: "outlook:1".into(),
            thread_id: "outlook:t".into(),
            folder: Folder::Inbox,
            from: Address::new(None, from),
            to: to.iter().filter_map(|a| Address::new(None, a)).collect(),
            cc: Vec::new(),
            subject: "s".into(),
            date: "2026-11-03T10:00:00Z".into(),
            preview: String::new(),
            bulk,
            unread: false,
            at: 0,
        }
    }

    fn named(name: &str, email: &str) -> Address {
        Address::new(Some(name), email).unwrap()
    }

    #[test]
    fn bulk_headers_are_recognised() {
        assert!(is_bulk_by_headers([(
            "List-Unsubscribe",
            "<mailto:u@x.com>"
        )]));
        assert!(is_bulk_by_headers([("Precedence", "Bulk")]));
        assert!(is_bulk_by_headers([("Auto-Submitted", "auto-generated")]));
        assert!(!is_bulk_by_headers([("Auto-Submitted", "no")]));
        assert!(!is_bulk_by_headers([("Subject", "List-Unsubscribe")]));
    }

    #[test]
    fn bulk_and_notes_to_self_are_left_out() {
        let filter = MailFilter::new(&["Dev@Company.com".into()], &[]);
        assert!(filter.keep(&header("lena@x.com", &["dev@company.com"], false)));
        assert!(!filter.keep(&header("news@x.com", &["dev@company.com"], true)));
        // Sent only to themselves
        assert!(!filter.keep(&header("dev@company.com", &["dev@company.com"], false)));
        // Sent by the user to someone else: kept (their commitments)
        assert!(filter.keep(&header("dev@company.com", &["lena@x.com"], false)));
    }

    #[test]
    fn watched_senders_always_come_through() {
        let filter = MailFilter::new(&[], &["@vendor.com".into(), "ceo@board.org".into()]);
        assert!(filter.keep(&header("billing@vendor.com", &[], true)));
        assert!(filter.keep(&header("a@eu.vendor.com", &[], true)));
        assert!(filter.keep(&header("ceo@board.org", &[], true)));
        assert!(!filter.keep(&header("other@board.org", &[], true)));
        assert!(!filter.keep(&header("x@notvendor.com", &[], true)));
    }

    #[test]
    fn machine_senders_are_recognised() {
        for sender in [
            "noreply@x.com",
            "no-reply@github.com",
            "notifications+abc@slack.com",
            "alerts-eu@aws.com",
            "mailer-daemon@googlemail.com",
        ] {
            assert!(is_automated(&named("", sender)), "{sender}");
        }
        for sender in ["lena@x.com", "newsdesk@paper.com", "systemsteam@x.com"] {
            assert!(!is_automated(&named("", sender)), "{sender}");
        }
    }

    #[test]
    fn phishing_senders_are_recognised() {
        assert!(looks_suspicious(&named(
            "PayPal Support",
            "help@secure-login.ru"
        )));
        assert!(looks_suspicious(&named("service@paypal.com", "a@evil.biz")));
        assert!(looks_suspicious(&named("Lena", "lena@xn--pple-43d.com")));
        assert!(looks_suspicious(&named(
            "Microsoft account team",
            "x@gmail.com"
        )));
        // The real thing, and people
        assert!(!looks_suspicious(&named("PayPal", "service@paypal.com")));
        assert!(!looks_suspicious(&named(
            "Microsoft",
            "no-reply@accountprotection.microsoft.com"
        )));
        assert!(!looks_suspicious(&named(
            "Amazon Web Services",
            "aws@amazon.com"
        )));
        assert!(!looks_suspicious(&named("Lena Park", "lena@contoso.com")));
        assert!(
            !looks_suspicious(&named("Groups team", "team@contoso.com")),
            "not UPS"
        );
        assert!(!looks_suspicious(&named(
            "lena@eu.contoso.com",
            "lena@contoso.com"
        )));
    }

    #[test]
    fn phishing_is_left_out_of_all_mail_unless_watched() {
        let filter = MailFilter::new(&[], &["secure-login.ru".into()]);
        let mut fake = header("help@evil.biz", &[], false);
        fake.from = Some(named("PayPal", "help@evil.biz"));
        assert!(!filter.keep(&fake));
        assert!(!filter.keep_older(&fake));
        fake.from = Some(named("PayPal", "help@secure-login.ru"));
        assert!(filter.keep(&fake), "watched");
    }

    #[test]
    fn older_mail_leaves_out_unread_and_machine_mail() {
        let filter = MailFilter::new(&[], &["@vendor.com".into()]);
        let person = header("lena@x.com", &[], false);
        assert!(filter.keep_older(&person));
        let unread = Header {
            unread: true,
            ..person.clone()
        };
        assert!(filter.keep(&unread), "new mail keeps it");
        assert!(!filter.keep_older(&unread), "never opened: not relevant");
        let machine = header("notifications@jira.com", &[], false);
        assert!(filter.keep(&machine), "new mail keeps it");
        assert!(!filter.keep_older(&machine));
        let watched = Header {
            unread: true,
            ..header("noreply@vendor.com", &[], false)
        };
        assert!(filter.keep_older(&watched));
    }
}
