//! Outlook and Microsoft 365 mail through Microsoft Graph (`Mail.Read`).
//!
//! Changes come from delta queries on Inbox and Sent Items
//! (`/me/mailFolders/{inbox,sentitems}/messages/delta`); the cursor is the
//! two `@odata.deltaLink`s. The first query asks only for mail received
//! since the window start, never the whole mailbox. A delta link Graph no
//! longer accepts (410) starts again from the window.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::api::{MailApi, MailError};
use super::filter::is_bulk_by_headers;
use super::mail::{rfc3339_of, str_of, unix_of, Changes, Folder, Header, Identity, Message};
use super::profile::Profile;
use super::text::Address;

/// Only these fields are read
const SELECT: &str = "id,subject,from,toRecipients,ccRecipients,receivedDateTime,\
conversationId,bodyPreview,inferenceClassification";
const FOLDERS: [(&str, Folder); 2] = [("inbox", Folder::Inbox), ("sentitems", Folder::Sent)];
/// Pages per folder per check, at most
const MAX_PAGES: usize = 40;
const PAGE_SIZE: (&str, &str) = ("Prefer", "odata.maxpagesize=50");
const PREFIX: &str = "outlook:";

/// The delta links, one per folder
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cursor {
    #[serde(default)]
    inbox: Option<String>,
    #[serde(default)]
    sentitems: Option<String>,
}

impl Cursor {
    fn link(&self, folder: &str) -> Option<&String> {
        match folder {
            "inbox" => self.inbox.as_ref(),
            _ => self.sentitems.as_ref(),
        }
    }

    fn set(&mut self, folder: &str, link: String) {
        match folder {
            "inbox" => self.inbox = Some(link),
            _ => self.sentitems = Some(link),
        }
    }
}

pub struct Outlook<'a> {
    api: &'a MailApi,
    graph: &'a str,
}

impl<'a> Outlook<'a> {
    pub fn new(api: &'a MailApi, graph: &'a str) -> Self {
        Self { api, graph }
    }

    fn url(&self, path: &[&str], params: &[(&str, &str)]) -> String {
        self.api.url(self.graph, path, params)
    }

    pub async fn identity(&self) -> Result<Identity, MailError> {
        let me = self
            .api
            .get(
                &self.url(
                    &["me"],
                    &[("$select", "displayName,mail,userPrincipalName")],
                ),
                &[],
            )
            .await?;
        let email = [str_of(&me, "mail"), str_of(&me, "userPrincipalName")]
            .into_iter()
            .find(|e| e.contains('@'))
            .unwrap_or_default()
            .to_ascii_lowercase();
        if email.is_empty() {
            return Err(MailError::Failed(
                "Microsoft didn't say which mailbox this is".into(),
            ));
        }
        let name = Some(str_of(&me, "displayName").trim().to_string()).filter(|n| !n.is_empty());
        Ok(Identity { name, email })
    }

    /// Job title, department and manager (`/me`, `/me/manager`, inside
    /// `User.Read`). A failed read leaves that part empty.
    pub async fn profile(&self) -> Profile {
        let me = self
            .api
            .get(
                &self.url(&["me"], &[("$select", "jobTitle,department")]),
                &[],
            )
            .await
            .map(|me| Profile::from_me(&me))
            .unwrap_or_else(|e| {
                log::debug!("Outlook title and department unavailable: {e}");
                Profile::default()
            });
        let manager = self
            .api
            .get(
                &self.url(
                    &["me", "manager"],
                    &[("$select", "displayName,mail,userPrincipalName,jobTitle")],
                ),
                &[],
            )
            .await;
        match manager {
            Ok(manager) => me.with_manager(&manager),
            Err(e) => {
                log::debug!("Outlook manager unavailable: {e}");
                me
            }
        }
    }

    /// Mail added to Inbox and Sent Items since `cursor` (or since `since`
    /// without one)
    pub async fn changes(&self, cursor: &Value, since: i64) -> Result<Changes, MailError> {
        let mut stored: Cursor = serde_json::from_value(cursor.clone()).unwrap_or_default();
        let mut headers = Vec::new();
        let mut restarted = false;
        for (folder, kind) in FOLDERS {
            let start = self.url(
                &["me", "mailFolders", folder, "messages", "delta"],
                &[
                    ("$select", SELECT),
                    (
                        "$filter",
                        &format!("receivedDateTime ge {}", rfc3339_of(since)),
                    ),
                ],
            );
            let (found, link) = match stored.link(folder) {
                Some(link) => match self.delta(link, kind).await {
                    Err(MailError::CursorExpired) => {
                        log::info!("The Outlook {folder} delta link expired; reading the window");
                        restarted = true;
                        self.delta(&start, kind).await?
                    }
                    other => other?,
                },
                None => {
                    restarted = true;
                    self.delta(&start, kind).await?
                }
            };
            headers.extend(found);
            stored.set(folder, link);
        }
        // A delta link catching up after a long time: still only the window
        headers.retain(|h| h.at >= since);
        Ok(Changes {
            headers,
            cursor: serde_json::to_value(&stored).unwrap_or_default(),
            restarted,
        })
    }

    /// Follows `url` page by page to its delta link
    async fn delta(&self, url: &str, folder: Folder) -> Result<(Vec<Header>, String), MailError> {
        let mut headers = Vec::new();
        let mut next = url.to_string();
        for _ in 0..MAX_PAGES {
            let page = self.api.get(&next, &[PAGE_SIZE]).await?;
            headers.extend(items(&page).filter_map(|item| header(item, folder)));
            if let Some(link) = page.get("@odata.deltaLink").and_then(Value::as_str) {
                return Ok((headers, link.to_string()));
            }
            match page.get("@odata.nextLink").and_then(Value::as_str) {
                Some(link) => next = link.to_string(),
                None => break,
            }
        }
        Err(MailError::Unavailable(
            "Outlook's change list didn't finish; Nandi tries again next check".into(),
        ))
    }

    /// Marks inbox mail sent to a list (`List-Unsubscribe` and the like) as
    /// bulk. Graph has these headers only per message.
    pub async fn mark_bulk(&self, headers: &mut [Header]) {
        for header in headers
            .iter_mut()
            .filter(|h| h.folder == Folder::Inbox && !h.bulk)
        {
            let Some(id) = header.id.strip_prefix(PREFIX) else {
                continue;
            };
            let url = self.url(
                &["me", "messages", id],
                &[("$select", "internetMessageHeaders")],
            );
            match self.api.get(&url, &[]).await {
                Ok(message) => {
                    let pairs = message
                        .get("internetMessageHeaders")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|h| (str_of(h, "name"), str_of(h, "value")));
                    header.bulk = is_bulk_by_headers(pairs);
                }
                Err(e) => log::debug!("Outlook headers unavailable for one message: {e}"),
            }
        }
    }

    pub async fn message(&self, id: &str) -> Result<Message, MailError> {
        let item = self
            .api
            .get(
                &self.url(
                    &["me", "messages", id],
                    &[("$select", &format!("{SELECT},body,internetMessageHeaders"))],
                ),
                &[("Prefer", "outlook.body-content-type=\"text\"")],
            )
            .await?;
        let header = header(&item, Folder::Other).ok_or(MailError::NotFound)?;
        let body = item
            .get("body")
            .map(|b| str_of(b, "content"))
            .unwrap_or_default();
        let headers = item
            .get("internetMessageHeaders")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|h| (str_of(h, "name"), str_of(h, "value")));
        let results = super::mail::authentication_results(headers);
        Ok(Message::new(header, body).with_authentication(&results))
    }

    pub async fn thread(
        &self,
        conversation_id: &str,
        limit: u32,
    ) -> Result<Vec<Header>, MailError> {
        let filter = format!(
            "conversationId eq '{}'",
            conversation_id.replace('\'', "''")
        );
        let top = limit.to_string();
        let page = self
            .api
            .get(
                &self.url(
                    &["me", "messages"],
                    &[("$filter", &filter), ("$select", SELECT), ("$top", &top)],
                ),
                &[],
            )
            .await?;
        let mut headers: Vec<Header> = items(&page)
            .filter_map(|item| header(item, Folder::Other))
            .collect();
        headers.sort_by_key(|h| h.at);
        Ok(headers)
    }

    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<Header>, MailError> {
        let search = format!("\"{}\"", query.replace('"', " "));
        let top = limit.to_string();
        let page = self
            .api
            .get(
                &self.url(
                    &["me", "messages"],
                    &[("$search", &search), ("$select", SELECT), ("$top", &top)],
                ),
                &[],
            )
            .await?;
        Ok(items(&page)
            .filter_map(|item| header(item, Folder::Other))
            .collect())
    }

    /// Inbox and Sent Items mail older than `before` (and not older than
    /// `floor`), newest first, at most `limit`: one background-read batch
    pub async fn older(
        &self,
        before: i64,
        floor: Option<i64>,
        limit: u32,
    ) -> Result<Vec<Header>, MailError> {
        let mut filter = format!("receivedDateTime lt {}", rfc3339_of(before));
        if let Some(floor) = floor {
            filter.push_str(&format!(" and receivedDateTime ge {}", rfc3339_of(floor)));
        }
        let top = limit.to_string();
        let mut headers = Vec::new();
        for (folder, kind) in FOLDERS {
            let page = self
                .api
                .get(
                    &self.url(
                        &["me", "mailFolders", folder, "messages"],
                        &[
                            ("$filter", &filter),
                            ("$orderby", "receivedDateTime desc"),
                            ("$select", SELECT),
                            ("$top", &top),
                        ],
                    ),
                    &[],
                )
                .await?;
            headers.extend(items(&page).filter_map(|item| header(item, kind)));
        }
        headers.sort_by_key(|h| std::cmp::Reverse(h.at));
        headers.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        Ok(headers)
    }

    /// About how many Inbox and Sent Items emails are older than `before`
    /// and not older than `floor` (`$count`, eventual consistency)
    pub async fn count(&self, before: i64, floor: Option<i64>) -> Result<u64, MailError> {
        let mut filter = format!("receivedDateTime lt {}", rfc3339_of(before));
        if let Some(floor) = floor {
            filter.push_str(&format!(" and receivedDateTime ge {}", rfc3339_of(floor)));
        }
        let mut total = 0;
        for (folder, _) in FOLDERS {
            let page = self
                .api
                .get(
                    &self.url(
                        &["me", "mailFolders", folder, "messages"],
                        &[
                            ("$count", "true"),
                            ("$top", "1"),
                            ("$select", "id"),
                            ("$filter", &filter),
                        ],
                    ),
                    &[("ConsistencyLevel", "eventual")],
                )
                .await?;
            total += page
                .get("@odata.count")
                .and_then(Value::as_u64)
                .unwrap_or(0);
        }
        Ok(total)
    }

    /// Inbox and Sent Items mail since `since`, newest first
    pub async fn recent(&self, since: i64, limit: u32) -> Result<Vec<Header>, MailError> {
        let filter = format!("receivedDateTime ge {}", rfc3339_of(since));
        let top = limit.to_string();
        let mut headers = Vec::new();
        for (folder, kind) in FOLDERS {
            let page = self
                .api
                .get(
                    &self.url(
                        &["me", "mailFolders", folder, "messages"],
                        &[
                            ("$filter", &filter),
                            ("$orderby", "receivedDateTime desc"),
                            ("$select", SELECT),
                            ("$top", &top),
                        ],
                    ),
                    &[],
                )
                .await?;
            headers.extend(items(&page).filter_map(|item| header(item, kind)));
        }
        headers.sort_by_key(|h| std::cmp::Reverse(h.at));
        Ok(headers)
    }
}

fn items(page: &Value) -> impl Iterator<Item = &Value> {
    page.get("value")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn address(value: &Value) -> Option<Address> {
    let inner = value.get("emailAddress")?;
    Address::new(
        inner.get("name").and_then(Value::as_str),
        str_of(inner, "address"),
    )
}

fn addresses(value: &Value, key: &str) -> Vec<Address> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(address)
        .collect()
}

/// A Graph message as a header; `None` for a removal (`@removed`)
fn header(item: &Value, folder: Folder) -> Option<Header> {
    if item.get("@removed").is_some() {
        return None;
    }
    let id = str_of(item, "id");
    if id.is_empty() {
        return None;
    }
    let date = str_of(item, "receivedDateTime");
    Some(
        Header {
            id: format!("{PREFIX}{id}"),
            thread_id: format!("{PREFIX}{}", str_of(item, "conversationId")),
            folder,
            from: item.get("from").and_then(address),
            to: addresses(item, "toRecipients"),
            cc: addresses(item, "ccRecipients"),
            subject: str_of(item, "subject").to_string(),
            date: date.to_string(),
            preview: str_of(item, "bodyPreview").to_string(),
            bulk: str_of(item, "inferenceClassification").eq_ignore_ascii_case("other"),
            at: unix_of(date),
        }
        .tidy(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_a_graph_message() {
        let item = json!({
            "id": "AAMk1",
            "conversationId": "conv-1",
            "subject": "  Q3   audit evidence ",
            "from": {"emailAddress": {"name": "Lena Park", "address": "Lena@Contoso.com"}},
            "toRecipients": [{"emailAddress": {"name": "Dev", "address": "dev@company.com"}}],
            "receivedDateTime": "2026-11-03T14:02:00Z",
            "bodyPreview": "Could you send the evidence list by Friday?",
            "inferenceClassification": "focused",
        });
        let header = header(&item, Folder::Inbox).unwrap();
        assert_eq!(header.id, "outlook:AAMk1");
        assert_eq!(header.thread_id, "outlook:conv-1");
        assert_eq!(header.subject, "Q3 audit evidence");
        assert_eq!(header.from.unwrap().email, "lena@contoso.com");
        assert_eq!(header.to[0].name.as_deref(), Some("Dev"));
        assert!(!header.bulk);
        assert_eq!(header.at, unix_of("2026-11-03T14:02:00Z"));
    }

    #[test]
    fn other_mail_is_bulk_and_removals_are_skipped() {
        let other = json!({"id": "x", "inferenceClassification": "other", "receivedDateTime": "2026-11-03T14:02:00Z"});
        assert!(header(&other, Folder::Inbox).unwrap().bulk);
        assert!(header(
            &json!({"id": "x", "@removed": {"reason": "deleted"}}),
            Folder::Inbox
        )
        .is_none());
    }
}
