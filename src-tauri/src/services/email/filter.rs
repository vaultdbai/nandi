//! What Nandi never sees: bulk mail (newsletters and
//! marketing, by the provider's category or the message's list headers) and
//! mail the user sent only to themselves. Senders and domains on the watch
//! list always come through.
//!
//! Everything else reaches Nandi, with the facts it needs to
//! judge it (unread, `List-Id`, `Auto-Submitted`: see `batch`). Whether a
//! sender is a machine or looks like phishing is the model's call, not a
//! word list's.

use super::mail::Header;
use super::text::Address;

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

    /// Whether Nandi should see `header`
    pub fn keep(&self, header: &Header) -> bool {
        let sender_watched = header.from.as_ref().is_some_and(|f| self.watched(f));
        if sender_watched {
            return true;
        }
        if header.bulk {
            return false;
        }
        let from_me = header.from.as_ref().is_some_and(|f| self.is_me(f));
        let recipients: Vec<&Address> = header.to.iter().chain(&header.cc).collect();
        let only_to_me = !recipients.is_empty() && recipients.iter().all(|a| self.is_me(a));
        !(from_me && only_to_me)
    }
}

#[cfg(test)]
mod tests {
    use super::super::mail::Folder;
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
            list_id: None,
            auto_submitted: None,
            at: 0,
        }
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
    fn machine_unread_and_phishing_looking_mail_is_left_to_the_model() {
        let filter = MailFilter::new(&["dev@company.com".into()], &[]);
        let unread = Header {
            unread: true,
            ..header("lena@x.com", &["dev@company.com"], false)
        };
        let machine = Header {
            auto_submitted: Some("auto-generated".into()),
            list_id: Some("<builds.ci.x.com>".into()),
            ..header("noreply@ci.x.com", &["dev@company.com"], false)
        };
        let mut lookalike = header("help@secure-login.ru", &["dev@company.com"], false);
        lookalike.from = Address::new(Some("PayPal Support"), "help@secure-login.ru");
        for mail in [unread, machine, lookalike] {
            assert!(filter.keep(&mail), "{:?}", mail.from);
        }
    }
}
