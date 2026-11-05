//! What the Chief of Staff never sees: bulk mail (newsletters,
//! notifications) and mail the user sent only to themselves. Senders and
//! domains on the watch list always come through.

use super::mail::Header;
use super::text::Address;

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
            at: 0,
        }
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
}
