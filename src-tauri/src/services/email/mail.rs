//! The one shape both providers' mail comes in, so the Chief of Staff
//! doesn't care which mailbox is behind it. Nothing here is stored: a
//! header lives for one check or one tool call.

use serde::Serialize;
use serde_json::Value;

use super::text::{self, Address};

/// Longest preview a header carries
pub const PREVIEW_CHARS: usize = 240;
/// Longest subject
pub const SUBJECT_CHARS: usize = 200;
/// Longest body `mail_get_message` returns
pub const BODY_CHARS: usize = 4_000;
/// Longest `List-Id` or `Auto-Submitted` value a header carries
pub const LIST_HEADER_CHARS: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Folder {
    Inbox,
    Sent,
    Other,
}

/// One message's headers and preview
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Header {
    /// `outlook:<id>` or `gmail:<id>`
    pub id: String,
    /// `outlook:<conversationId>` or `gmail:<threadId>`
    pub thread_id: String,
    pub folder: Folder,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Address>,
    pub to: Vec<Address>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cc: Vec<Address>,
    pub subject: String,
    /// RFC 3339
    pub date: String,
    pub preview: String,
    /// Newsletters and marketing, by the provider's category or the
    /// message's list headers ([`ListHeaders::bulk`]). Only watched senders'
    /// bulk mail reaches the Chief of Staff (`filter`).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub bulk: bool,
    /// Never opened by the user (never true for sent mail)
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unread: bool,
    /// The mailing list it came through (`List-Id`), as sent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_id: Option<String>,
    /// Sent by a machine (`Auto-Submitted` other than `no`), as sent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_submitted: Option<String>,
    /// Unix seconds, for ordering
    #[serde(skip)]
    pub at: i64,
}

impl Header {
    /// The subject and preview, cut to size
    pub fn tidy(mut self) -> Self {
        self.subject = text::one_line(&self.subject, SUBJECT_CHARS);
        self.preview = text::one_line(&self.preview, PREVIEW_CHARS);
        self
    }

    /// Adds what the message's own list headers say. The user's own sent
    /// mail is never bulk and carries none of them.
    pub fn add_list_headers(&mut self, list: ListHeaders) {
        if self.folder == Folder::Sent {
            return;
        }
        self.bulk |= list.bulk;
        self.list_id = list.list_id;
        self.auto_submitted = list.auto_submitted;
    }
}

/// What a message's list headers say about how it was sent. Facts read as
/// sent: Nandi leaves out declared bulk mail, and the Chief of Staff weighs
/// the rest itself (`batch`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ListHeaders {
    /// Sent in bulk: a `List-Unsubscribe` header, or `Precedence: bulk`,
    /// `list` or `junk`
    pub bulk: bool,
    /// `List-Id`: sent through a mailing list, which may be a team's own
    pub list_id: Option<String>,
    /// `Auto-Submitted` other than `no`: sent by a machine (`auto-generated`,
    /// `auto-replied` …)
    pub auto_submitted: Option<String>,
}

impl ListHeaders {
    /// Reads the list headers among a message's headers. Pure.
    pub fn read<'a>(headers: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut list = Self::default();
        for (name, value) in headers {
            let value = text::one_line(value, LIST_HEADER_CHARS);
            if value.is_empty() {
                continue;
            }
            match name.to_ascii_lowercase().as_str() {
                "list-unsubscribe" => list.bulk = true,
                "precedence" => {
                    list.bulk |= matches!(
                        value.to_ascii_lowercase().as_str(),
                        "bulk" | "list" | "junk"
                    );
                }
                "list-id" => list.list_id = Some(value),
                "auto-submitted" if !value.eq_ignore_ascii_case("no") => {
                    list.auto_submitted = Some(value);
                }
                _ => {}
            }
        }
        list
    }
}

/// A message with its text, size capped. Quoted replies stay in: the
/// Chief of Staff tells the new part from the old itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Message {
    #[serde(flatten)]
    pub header: Header,
    pub body: String,
    /// The body was longer than [`BODY_CHARS`]
    pub truncated: bool,
    /// The sender passed DMARC for the From domain, by the receiving
    /// provider's `Authentication-Results` ([`dmarc_verified`]). Only
    /// verified mail can match an auto-approve rule (`intake::rules`).
    #[serde(skip)]
    pub verified_sender: bool,
}

impl Message {
    pub fn new(header: Header, raw_body: &str) -> Self {
        let (body, truncated) = text::cap(raw_body.trim(), BODY_CHARS);
        Self {
            header: header.tidy(),
            body,
            truncated,
            verified_sender: false,
        }
    }

    /// Sets [`Message::verified_sender`] from the message's
    /// `Authentication-Results` headers
    pub fn with_authentication(mut self, results: &[&str]) -> Self {
        self.verified_sender = self
            .header
            .from
            .as_ref()
            .is_some_and(|from| dmarc_verified(results, &from.email));
        self
    }
}

/// The `Authentication-Results` values among a message's headers, in order
pub fn authentication_results<'a>(
    headers: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Vec<&'a str> {
    headers
        .into_iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("Authentication-Results"))
        .map(|(_, value)| value)
        .collect()
}

/// The receiving provider says DMARC passed for the From domain: the first
/// `Authentication-Results` header (the one the user's provider added; ones
/// further down came with the mail and prove nothing) has `dmarc=pass` with
/// `header.from=<that domain>`. Pure.
pub fn dmarc_verified(results: &[&str], from: &str) -> bool {
    let Some(domain) = from.rsplit_once('@').map(|(_, d)| d.trim().to_lowercase()) else {
        return false;
    };
    let Some(first) = results.first() else {
        return false;
    };
    first.to_lowercase().split(';').any(|clause| {
        let words: Vec<&str> = clause.split_whitespace().collect();
        words.first() == Some(&"dmarc=pass")
            && words
                .iter()
                .any(|w| w.trim_end_matches(',') == format!("header.from={domain}"))
    })
}

/// What changed in a mailbox since its cursor
#[derive(Debug, Clone, PartialEq)]
pub struct Changes {
    pub headers: Vec<Header>,
    /// The cursor to store once the batch is handed over
    pub cursor: Value,
    /// No usable cursor (first check, or it expired): read the recent
    /// window instead
    pub restarted: bool,
}

/// Unix seconds of an RFC 3339 time
pub fn unix_of(rfc3339: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .map(|t| t.timestamp())
        .unwrap_or_default()
}

/// RFC 3339 of unix seconds
pub fn rfc3339_of(unix: i64) -> String {
    chrono::DateTime::from_timestamp(unix, 0)
        .unwrap_or_default()
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A JSON string field, or empty
pub fn str_of<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Who a mailbox belongs to
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: Option<String>,
    pub email: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(folder: Folder) -> Header {
        Header {
            id: "gmail:1".into(),
            thread_id: "gmail:t".into(),
            folder,
            from: None,
            to: Vec::new(),
            cc: Vec::new(),
            subject: String::new(),
            date: String::new(),
            preview: String::new(),
            bulk: false,
            unread: false,
            list_id: None,
            auto_submitted: None,
            at: 0,
        }
    }

    #[test]
    fn list_headers_say_bulk_list_and_machine() {
        assert!(ListHeaders::read([("List-Unsubscribe", "<mailto:u@x.com>")]).bulk);
        assert!(ListHeaders::read([("Precedence", "Bulk")]).bulk);
        assert!(!ListHeaders::read([("Precedence", "first-class")]).bulk);
        assert!(!ListHeaders::read([("Subject", "List-Unsubscribe")]).bulk);

        // A mailing list and a machine sender are facts, not bulk
        let team = ListHeaders::read([
            ("List-Id", "Platform team <platform.contoso.com>"),
            ("Auto-Submitted", "auto-generated"),
        ]);
        assert!(!team.bulk);
        assert_eq!(
            team.list_id.as_deref(),
            Some("Platform team <platform.contoso.com>")
        );
        assert_eq!(team.auto_submitted.as_deref(), Some("auto-generated"));
        assert_eq!(
            ListHeaders::read([("Auto-Submitted", "no")]).auto_submitted,
            None
        );
    }

    #[test]
    fn list_header_values_are_one_short_line() {
        let long = format!("a\n{}", "x".repeat(500));
        let list = ListHeaders::read([("List-Id", long.as_str())]);
        let id = list.list_id.unwrap();
        assert!(!id.contains('\n'));
        assert_eq!(id.chars().count(), LIST_HEADER_CHARS);
    }

    #[test]
    fn the_users_sent_mail_takes_no_list_headers() {
        let mut header = header(Folder::Sent);
        let list = ListHeaders::read([("List-Unsubscribe", "<x>"), ("List-Id", "<l.x.com>")]);
        header.add_list_headers(list.clone());
        assert!(!header.bulk && header.list_id.is_none());
        header.folder = Folder::Inbox;
        header.add_list_headers(list);
        assert!(header.bulk);
        assert_eq!(header.list_id.as_deref(), Some("<l.x.com>"));
    }

    #[test]
    fn a_body_keeps_its_quoted_reply_and_is_capped() {
        let header = header(Folder::Inbox);
        let reply = "Approved.\n\nOn Mon, Sam wrote:\n> Please approve a < b";
        let message = Message::new(header.clone(), reply);
        assert_eq!(message.body, reply, "the model reads the quote itself");
        assert!(!message.truncated);
        let long = Message::new(header, &"word ".repeat(2_000));
        assert!(long.truncated);
        assert_eq!(long.body.chars().count(), BODY_CHARS);
    }
}
