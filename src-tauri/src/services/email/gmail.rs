//! Gmail through the Gmail API (`gmail.readonly`).
//!
//! The cursor is the mailbox's `historyId`. The first check (or one whose
//! history is too old: 404) saves the current `historyId` from the profile
//! and lists the window's inbox and sent mail; later checks read
//! `users.history.list` for added messages. Promotions, Social, Updates and
//! Forums count as bulk, as does mail with list headers.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde_json::{json, Value};

use super::api::{MailApi, MailError};
use super::filter::is_bulk_by_headers;
use super::mail::{rfc3339_of, str_of, Changes, Folder, Header, Identity, Message};
use super::text::{self, parse_addresses};

const PREFIX: &str = "gmail:";
/// Headers read with each message, and nothing else
const METADATA: &[&str] = &[
    "From",
    "To",
    "Cc",
    "Subject",
    "Date",
    "List-Unsubscribe",
    "List-Id",
    "Precedence",
    "Auto-Submitted",
];
const BULK_LABELS: &[&str] = &[
    "CATEGORY_PROMOTIONS",
    "CATEGORY_SOCIAL",
    "CATEGORY_UPDATES",
    "CATEGORY_FORUMS",
];
const SKIPPED_LABELS: &[&str] = &["DRAFT", "SPAM", "TRASH", "CHAT"];
/// Pages of IDs per check, and messages read, at most
const MAX_PAGES: usize = 10;
const MAX_MESSAGES: usize = 150;

pub struct Gmail<'a> {
    api: &'a MailApi,
    base: &'a str,
    userinfo: &'a str,
}

impl<'a> Gmail<'a> {
    pub fn new(api: &'a MailApi, base: &'a str, userinfo: &'a str) -> Self {
        Self {
            api,
            base,
            userinfo,
        }
    }

    fn url(&self, path: &[&str], params: &[(&str, &str)]) -> String {
        let mut full = vec!["users", "me"];
        full.extend_from_slice(path);
        self.api.url(self.base, &full, params)
    }

    pub async fn identity(&self) -> Result<Identity, MailError> {
        let profile = self.api.get(&self.url(&["profile"], &[]), &[]).await?;
        let email = str_of(&profile, "emailAddress").to_ascii_lowercase();
        if email.is_empty() {
            return Err(MailError::Failed(
                "Google didn't say which mailbox this is".into(),
            ));
        }
        let name = match self.api.get(self.userinfo, &[]).await {
            Ok(info) => Some(str_of(&info, "name").trim().to_string()).filter(|n| !n.is_empty()),
            Err(e) => {
                log::debug!("Google profile name unavailable: {e}");
                None
            }
        };
        Ok(Identity { name, email })
    }

    async fn history_id(&self) -> Result<String, MailError> {
        let profile = self.api.get(&self.url(&["profile"], &[]), &[]).await?;
        match profile.get("historyId") {
            Some(Value::String(id)) => Ok(id.clone()),
            Some(Value::Number(id)) => Ok(id.to_string()),
            _ => Err(MailError::Failed("Gmail returned no history ID".into())),
        }
    }

    /// Inbox and sent mail added since `cursor` (`{"historyId": …}`), or
    /// since `since` without one
    pub async fn changes(&self, cursor: &Value, since: i64) -> Result<Changes, MailError> {
        let stored = cursor
            .get("historyId")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(start) = stored {
            match self.history(&start).await {
                Ok((ids, latest)) => {
                    let mut headers = self.headers(&ids).await?;
                    headers.retain(|h| h.at >= since);
                    return Ok(Changes {
                        headers,
                        cursor: json!({ "historyId": latest.unwrap_or(start) }),
                        restarted: false,
                    });
                }
                Err(MailError::NotFound | MailError::CursorExpired) => {
                    log::info!("The Gmail history ID is too old; reading the window");
                }
                Err(e) => return Err(e),
            }
        }
        // Taken first, so nothing that arrives while listing is missed
        let latest = self.history_id().await?;
        let ids = self.window(since).await?;
        Ok(Changes {
            headers: self.headers(&ids).await?,
            cursor: json!({ "historyId": latest }),
            restarted: true,
        })
    }

    /// IDs of inbox and sent mail added since history `start`, and the
    /// newest history ID
    async fn history(&self, start: &str) -> Result<(Vec<String>, Option<String>), MailError> {
        let mut ids = Vec::new();
        let mut latest = None;
        let mut page_token = String::new();
        for _ in 0..MAX_PAGES {
            let mut params = vec![
                ("startHistoryId", start),
                ("historyTypes", "messageAdded"),
                ("maxResults", "100"),
            ];
            if !page_token.is_empty() {
                params.push(("pageToken", &page_token));
            }
            let page = self.api.get(&self.url(&["history"], &params), &[]).await?;
            if let Some(id) = page.get("historyId") {
                latest = id
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| id.as_u64().map(|n| n.to_string()));
            }
            for entry in page
                .get("history")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                for added in entry
                    .get("messagesAdded")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let message = added.get("message").unwrap_or(&Value::Null);
                    let labels = labels(message);
                    let wanted = labels.iter().any(|l| *l == "INBOX" || *l == "SENT")
                        && !labels.iter().any(|l| SKIPPED_LABELS.contains(l));
                    let id = str_of(message, "id");
                    if wanted && !id.is_empty() && !ids.iter().any(|i| i == id) {
                        ids.push(id.to_string());
                    }
                }
            }
            match page.get("nextPageToken").and_then(Value::as_str) {
                Some(token) => page_token = token.to_string(),
                None => break,
            }
        }
        Ok((ids, latest))
    }

    /// IDs of inbox and sent mail since `since`, newest first
    async fn window(&self, since: i64) -> Result<Vec<String>, MailError> {
        self.list(&format!("after:{since} {}", mailbox_query()), MAX_MESSAGES)
            .await
    }

    async fn list(&self, query: &str, limit: usize) -> Result<Vec<String>, MailError> {
        let mut ids = Vec::new();
        let mut page_token = String::new();
        for _ in 0..MAX_PAGES {
            let mut params = vec![("q", query), ("maxResults", "100")];
            if !page_token.is_empty() {
                params.push(("pageToken", &page_token));
            }
            let page = self.api.get(&self.url(&["messages"], &params), &[]).await?;
            ids.extend(
                page.get("messages")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|m| str_of(m, "id").to_string())
                    .filter(|id| !id.is_empty()),
            );
            if ids.len() >= limit {
                ids.truncate(limit);
                break;
            }
            match page.get("nextPageToken").and_then(Value::as_str) {
                Some(token) => page_token = token.to_string(),
                None => break,
            }
        }
        Ok(ids)
    }

    /// The headers of `ids` (format `metadata`: the listed headers and the
    /// snippet only), newest first
    async fn headers(&self, ids: &[String]) -> Result<Vec<Header>, MailError> {
        let mut headers = Vec::new();
        let mut params: Vec<(&str, &str)> = vec![("format", "metadata")];
        params.extend(METADATA.iter().map(|h| ("metadataHeaders", *h)));
        for id in ids.iter().take(MAX_MESSAGES) {
            match self
                .api
                .get(&self.url(&["messages", id], &params), &[])
                .await
            {
                Ok(message) => headers.extend(header(&message)),
                // Deleted since it was listed
                Err(MailError::NotFound) => {}
                Err(e) => return Err(e),
            }
        }
        headers.sort_by_key(|h| std::cmp::Reverse(h.at));
        Ok(headers)
    }

    pub async fn message(&self, id: &str) -> Result<Message, MailError> {
        let message = self
            .api
            .get(&self.url(&["messages", id], &[("format", "full")]), &[])
            .await?;
        let header = header(&message).ok_or(MailError::NotFound)?;
        let body = body_text(message.get("payload").unwrap_or(&Value::Null));
        let headers = message
            .get("payload")
            .and_then(|p| p.get("headers"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|h| (str_of(h, "name"), str_of(h, "value")));
        let results = super::mail::authentication_results(headers);
        Ok(Message::new(header, &body).with_authentication(&results))
    }

    pub async fn thread(&self, thread_id: &str, limit: u32) -> Result<Vec<Header>, MailError> {
        let mut params: Vec<(&str, &str)> = vec![("format", "metadata")];
        params.extend(METADATA.iter().map(|h| ("metadataHeaders", *h)));
        let thread = self
            .api
            .get(&self.url(&["threads", thread_id], &params), &[])
            .await?;
        let mut headers: Vec<Header> = thread
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(header)
            .collect();
        headers.sort_by_key(|h| h.at);
        headers.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        Ok(headers)
    }

    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<Header>, MailError> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let ids = self.list(query, limit).await?;
        self.headers(&ids).await
    }

    /// Inbox and sent mail older than `before` (not older than `floor`),
    /// without the bulk categories, newest first: one background-read batch
    pub async fn older(
        &self,
        before: i64,
        floor: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Header>, MailError> {
        let ids = self
            .list(
                &older_query(before, floor),
                usize::try_from(limit).unwrap_or(usize::MAX),
            )
            .await?;
        self.headers(&ids).await
    }

    /// Gmail's estimate of how many such emails there are
    pub async fn count(&self, before: i64, floor: Option<i64>) -> Result<u64, MailError> {
        let page = self
            .api
            .get(
                &self.url(
                    &["messages"],
                    &[("q", &older_query(before, floor)), ("maxResults", "1")],
                ),
                &[],
            )
            .await?;
        Ok(page
            .get("resultSizeEstimate")
            .and_then(Value::as_u64)
            .unwrap_or(0))
    }

    /// Inbox and sent mail since `since`, newest first
    pub async fn recent(&self, since: i64, limit: u32) -> Result<Vec<Header>, MailError> {
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let ids = self
            .list(&format!("after:{since} {}", mailbox_query()), limit)
            .await?;
        self.headers(&ids).await
    }
}

/// Older mail for the background read: no bulk categories
fn older_query(before: i64, floor: Option<i64>) -> String {
    let mut query = format!(
        "before:{before} {} -category:promotions -category:social -category:updates -category:forums",
        mailbox_query()
    );
    if let Some(floor) = floor {
        query.push_str(&format!(" after:{floor}"));
    }
    query
}

/// Inbox and sent mail, nothing else
fn mailbox_query() -> &'static str {
    "(in:inbox OR in:sent) -in:chats -in:spam -in:trash -in:drafts"
}

fn labels(message: &Value) -> Vec<&str> {
    message
        .get("labelIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

/// A Gmail message (format `metadata` or `full`) as a header
fn header(message: &Value) -> Option<Header> {
    let id = str_of(message, "id");
    if id.is_empty() {
        return None;
    }
    let pairs: Vec<(&str, &str)> = message
        .get("payload")
        .and_then(|p| p.get("headers"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|h| (str_of(h, "name"), str_of(h, "value")))
        .collect();
    let get = |name: &str| {
        pairs
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map_or("", |(_, v)| *v)
    };
    let labels = labels(message);
    let at = str_of(message, "internalDate")
        .parse::<i64>()
        .map_or(0, |ms| ms / 1000);
    let folder = if labels.contains(&"SENT") {
        Folder::Sent
    } else if labels.contains(&"INBOX") {
        Folder::Inbox
    } else {
        Folder::Other
    };
    Some(
        Header {
            id: format!("{PREFIX}{id}"),
            thread_id: format!("{PREFIX}{}", str_of(message, "threadId")),
            folder,
            from: parse_addresses(get("From")).into_iter().next(),
            to: parse_addresses(get("To")),
            cc: parse_addresses(get("Cc")),
            subject: get("Subject").to_string(),
            date: rfc3339_of(at),
            preview: html_entities(str_of(message, "snippet")),
            bulk: folder != Folder::Sent
                && (labels.iter().any(|l| BULK_LABELS.contains(l))
                    || is_bulk_by_headers(pairs.iter().copied())),
            at,
        }
        .tidy(),
    )
}

/// Gmail snippets come HTML-escaped
fn html_entities(snippet: &str) -> String {
    snippet
        .replace("&#39;", "'")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The message's text: its first `text/plain` part, else its HTML as text
fn body_text(payload: &Value) -> String {
    fn find<'v>(part: &'v Value, mime: &str) -> Option<&'v str> {
        if str_of(part, "mimeType").eq_ignore_ascii_case(mime) {
            if let Some(data) = part
                .get("body")
                .and_then(|b| b.get("data"))
                .and_then(Value::as_str)
            {
                return Some(data);
            }
        }
        part.get("parts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(|child| find(child, mime))
    }
    let decode = |data: &str| {
        URL_SAFE_NO_PAD
            .decode(data.trim_end_matches('='))
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    };
    if let Some(plain) = find(payload, "text/plain").and_then(decode) {
        return plain;
    }
    find(payload, "text/html")
        .and_then(decode)
        .map(|html| text::html_to_text(&html))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata(id: &str, labels: &[&str], extra: &[(&str, &str)]) -> Value {
        let mut headers = vec![
            json!({"name": "From", "value": "Lena Park <lena@contoso.com>"}),
            json!({"name": "To", "value": "dev@company.com"}),
            json!({"name": "Subject", "value": "Evidence list"}),
        ];
        headers.extend(extra.iter().map(|(n, v)| json!({"name": n, "value": v})));
        json!({
            "id": id,
            "threadId": "t1",
            "labelIds": labels,
            "snippet": "Due Friday &amp; don&#39;t forget",
            "internalDate": "1793800000000",
            "payload": {"headers": headers},
        })
    }

    #[test]
    fn maps_gmail_metadata() {
        let header = header(&metadata("m1", &["INBOX", "IMPORTANT"], &[])).unwrap();
        assert_eq!(header.id, "gmail:m1");
        assert_eq!(header.thread_id, "gmail:t1");
        assert_eq!(header.folder, Folder::Inbox);
        assert_eq!(header.from.unwrap().name.as_deref(), Some("Lena Park"));
        assert_eq!(header.preview, "Due Friday & don't forget");
        assert_eq!(header.at, 1_793_800_000);
        assert!(!header.bulk);
    }

    #[test]
    fn categories_and_list_headers_are_bulk() {
        assert!(
            header(&metadata("m", &["INBOX", "CATEGORY_PROMOTIONS"], &[]))
                .unwrap()
                .bulk
        );
        assert!(
            header(&metadata(
                "m",
                &["INBOX"],
                &[("List-Unsubscribe", "<https://x>")]
            ))
            .unwrap()
            .bulk
        );
        // The user's own sent mail is never bulk
        assert!(
            !header(&metadata("m", &["SENT", "CATEGORY_UPDATES"], &[]))
                .unwrap()
                .bulk
        );
    }

    #[test]
    fn reads_the_plain_text_part_or_the_html_one() {
        let encode = |s: &str| URL_SAFE_NO_PAD.encode(s);
        let payload = json!({
            "mimeType": "multipart/alternative",
            "parts": [
                {"mimeType": "text/html", "body": {"data": encode("<p>Hi <b>there</b></p>")}},
                {"mimeType": "text/plain", "body": {"data": encode("Hi there")}},
            ]
        });
        assert_eq!(body_text(&payload), "Hi there");
        let html_only =
            json!({"mimeType": "text/html", "body": {"data": encode("<p>Hi <b>there</b></p>")}});
        assert_eq!(body_text(&html_only), "Hi there");
    }
}
