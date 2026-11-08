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
    /// Newsletters and notifications, by the provider's signals
    #[serde(skip)]
    pub bulk: bool,
    /// Never opened by the user (never true for sent mail)
    #[serde(skip)]
    pub unread: bool,
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
}

/// A message with its text, quoted replies cut and size capped
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
        let (body, truncated) = text::cap(&text::trim_quoted(raw_body), BODY_CHARS);
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
