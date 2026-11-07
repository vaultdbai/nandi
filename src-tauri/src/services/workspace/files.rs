//! Read-only files: OneDrive (and files shared with the person) and Google
//! Drive. List and search, read a document's text, and the change feed.
//!
//! | Service  | Recent / search                                     | Text                                                    | Change feed                                   |
//! | -------- | --------------------------------------------------- | ------------------------------------------------------- | --------------------------------------------- |
//! | OneDrive | `GET /me/drive/recent`, `GET /me/drive/search(q=…)` | `GET /me/drive/items/{id}/content` (text files)         | `GET /me/drive/root/delta`, then its deltaLink |
//! | Drive    | `GET files?orderBy=modifiedTime desc`, `q=fullText` | `files/{id}/export` (Google Docs), `?alt=media` (text)  | `changes/startPageToken`, then `changes`       |
//!
//! No copies: a file's text lives for one tool call; notes link back to the
//! file.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::accounts::AccountKind;
use crate::services::email::api::{MailApi, MailError};

/// The most text one read returns
pub const MAX_TEXT_CHARS: usize = 12_000;
const MAX_DOWNLOAD_BYTES: usize = 1024 * 1024;
const MAX_PAGES: usize = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FileItem {
    /// `<connector>:<id>`
    pub id: String,
    pub name: String,
    pub mime_type: Option<String>,
    pub modified: Option<String>,
    pub modified_by: Option<String>,
    pub size: Option<u64>,
    pub link: Option<String>,
    pub folder: bool,
    /// Gone (deleted or unshared), in a change feed
    pub removed: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileChanges {
    pub items: Vec<FileItem>,
    pub cursor: Value,
}

fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut at = value;
    for key in path {
        at = at.get(*key)?;
    }
    at.as_str().filter(|s| !s.is_empty())
}

pub fn onedrive_item(item: &Value) -> Option<FileItem> {
    // Items shared with the person carry the real one in remoteItem
    let real = item.get("remoteItem").unwrap_or(item);
    let raw = str_at(real, &["id"]).or_else(|| str_at(item, &["id"]))?;
    Some(FileItem {
        id: format!("{}:{raw}", AccountKind::OneDrive.id()),
        name: str_at(item, &["name"]).unwrap_or_default().to_string(),
        mime_type: str_at(real, &["file", "mimeType"]).map(str::to_string),
        modified: str_at(item, &["lastModifiedDateTime"]).map(str::to_string),
        modified_by: str_at(item, &["lastModifiedBy", "user", "displayName"]).map(str::to_string),
        size: real.get("size").and_then(Value::as_u64),
        link: str_at(item, &["webUrl"]).map(str::to_string),
        folder: real.get("folder").is_some(),
        removed: item.get("deleted").is_some(),
    })
}

pub fn drive_item(item: &Value) -> Option<FileItem> {
    let raw = str_at(item, &["id"])?;
    let mime = str_at(item, &["mimeType"]);
    Some(FileItem {
        id: format!("{}:{raw}", AccountKind::GoogleDrive.id()),
        name: str_at(item, &["name"]).unwrap_or_default().to_string(),
        mime_type: mime.map(str::to_string),
        modified: str_at(item, &["modifiedTime"]).map(str::to_string),
        modified_by: str_at(item, &["lastModifyingUser", "displayName"]).map(str::to_string),
        size: str_at(item, &["size"]).and_then(|s| s.parse().ok()),
        link: str_at(item, &["webViewLink"]).map(str::to_string),
        folder: mime == Some("application/vnd.google-apps.folder"),
        removed: item
            .get("trashed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// Whether a file's bytes are text Nandi can hand over as is
pub fn is_text(name: &str, mime: Option<&str>) -> bool {
    let lower = name.to_ascii_lowercase();
    mime.is_some_and(|m| m.starts_with("text/") || m == "application/json")
        || [
            ".txt", ".md", ".csv", ".json", ".log", ".yaml", ".yml", ".tsv",
        ]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// The export type of a Google Docs file, when it has a text form
fn google_export(mime: &str) -> Option<&'static str> {
    match mime {
        "application/vnd.google-apps.document" | "application/vnd.google-apps.presentation" => {
            Some("text/plain")
        }
        "application/vnd.google-apps.spreadsheet" => Some("text/csv"),
        _ => None,
    }
}

const DRIVE_FIELDS: &str =
    "id,name,mimeType,modifiedTime,size,webViewLink,trashed,lastModifyingUser(displayName)";

pub struct Drive<'a> {
    pub kind: AccountKind,
    pub api: &'a MailApi,
    pub base: &'a str,
}

impl Drive<'_> {
    fn is_google(&self) -> bool {
        self.kind == AccountKind::GoogleDrive
    }

    fn items(&self, page: &Value) -> Vec<FileItem> {
        let key = if self.is_google() { "files" } else { "value" };
        page.get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|i| {
                        if self.is_google() {
                            drive_item(i)
                        } else {
                            onedrive_item(i)
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub async fn recent(&self, limit: u32) -> Result<Vec<FileItem>, MailError> {
        let top = limit.to_string();
        let fields = format!("files({DRIVE_FIELDS})");
        let url = if self.is_google() {
            self.api.url(
                self.base,
                &["files"],
                &[
                    ("orderBy", "modifiedTime desc"),
                    ("pageSize", top.as_str()),
                    ("q", "trashed = false"),
                    ("fields", fields.as_str()),
                ],
            )
        } else {
            self.api.url(
                self.base,
                &["me", "drive", "recent"],
                &[("$top", top.as_str())],
            )
        };
        Ok(self.items(&self.api.get(&url, &[]).await?))
    }

    /// Words in names and content; OneDrive's search includes files shared
    /// with the person
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<FileItem>, MailError> {
        let top = limit.to_string();
        let url = if self.is_google() {
            let escaped = query.replace('\\', "\\\\").replace('\'', "\\'");
            let q = format!("fullText contains '{escaped}' and trashed = false");
            let fields = format!("files({DRIVE_FIELDS})");
            self.api.url(
                self.base,
                &["files"],
                &[
                    ("q", q.as_str()),
                    ("pageSize", top.as_str()),
                    ("fields", fields.as_str()),
                ],
            )
        } else {
            let escaped = query.replace('\'', "''");
            self.api.url(
                self.base,
                &["me", "drive", &format!("search(q='{escaped}')")],
                &[("$top", top.as_str())],
            )
        };
        Ok(self.items(&self.api.get(&url, &[]).await?))
    }

    /// A file's text (capped), or why there is none
    pub async fn text(&self, id: &str) -> Result<(FileItem, Option<String>), MailError> {
        if self.is_google() {
            let meta_url = self
                .api
                .url(self.base, &["files", id], &[("fields", DRIVE_FIELDS)]);
            let meta = self.api.get(&meta_url, &[]).await?;
            let item = drive_item(&meta).ok_or(MailError::NotFound)?;
            let mime = item.mime_type.clone().unwrap_or_default();
            let url = if let Some(export) = google_export(&mime) {
                self.api
                    .url(self.base, &["files", id, "export"], &[("mimeType", export)])
            } else if is_text(&item.name, Some(&mime)) {
                self.api.url(self.base, &["files", id], &[("alt", "media")])
            } else {
                return Ok((item, None));
            };
            let text = self.api.get_text(&url, MAX_DOWNLOAD_BYTES).await?;
            return Ok((item, Some(cap(text))));
        }
        let meta_url = self.api.url(self.base, &["me", "drive", "items", id], &[]);
        let meta = self.api.get(&meta_url, &[]).await?;
        let item = onedrive_item(&meta).ok_or(MailError::NotFound)?;
        if item.folder
            || !is_text(&item.name, item.mime_type.as_deref())
            || item.size.is_some_and(|s| s as usize > MAX_DOWNLOAD_BYTES)
        {
            return Ok((item, None));
        }
        let url = self
            .api
            .url(self.base, &["me", "drive", "items", id, "content"], &[]);
        let text = self.api.get_text(&url, MAX_DOWNLOAD_BYTES).await?;
        Ok((item, Some(cap(text))))
    }

    /// What changed since `cursor` (`Null`: start now, return nothing)
    pub async fn changes(&self, cursor: &Value) -> Result<FileChanges, MailError> {
        let link = cursor.get("link").and_then(Value::as_str);
        let result = match link {
            Some(link) => self.read_changes(link.to_string()).await,
            None => self.start().await,
        };
        match result {
            Err(MailError::CursorExpired | MailError::NotFound) if link.is_some() => {
                self.start().await
            }
            other => other,
        }
    }

    /// A fresh cursor at "now" (the first read lists nothing)
    async fn start(&self) -> Result<FileChanges, MailError> {
        if self.is_google() {
            let url = self.api.url(self.base, &["changes", "startPageToken"], &[]);
            let page = self.api.get(&url, &[]).await?;
            let token = str_at(&page, &["startPageToken"]).ok_or(MailError::NotFound)?;
            return Ok(FileChanges {
                items: Vec::new(),
                cursor: serde_json::json!({ "link": self.changes_url(token) }),
            });
        }
        let url = self.api.url(
            self.base,
            &["me", "drive", "root", "delta"],
            &[("token", "latest")],
        );
        let page = self.api.get(&url, &[]).await?;
        let link = str_at(&page, &["@odata.deltaLink"]).ok_or(MailError::NotFound)?;
        Ok(FileChanges {
            items: Vec::new(),
            cursor: serde_json::json!({ "link": link }),
        })
    }

    fn changes_url(&self, token: &str) -> String {
        let fields =
            format!("nextPageToken,newStartPageToken,changes(fileId,removed,file({DRIVE_FIELDS}))");
        self.api.url(
            self.base,
            &["changes"],
            &[("pageToken", token), ("fields", fields.as_str())],
        )
    }

    async fn read_changes(&self, mut next: String) -> Result<FileChanges, MailError> {
        let mut items = Vec::new();
        for _ in 0..MAX_PAGES {
            let page = self.api.get(&next, &[]).await?;
            if self.is_google() {
                for change in page
                    .get("changes")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let removed = change
                        .get("removed")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let item = change.get("file").and_then(drive_item).or_else(|| {
                        str_at(change, &["fileId"]).map(|id| FileItem {
                            id: format!("{}:{id}", AccountKind::GoogleDrive.id()),
                            name: String::new(),
                            mime_type: None,
                            modified: None,
                            modified_by: None,
                            size: None,
                            link: None,
                            folder: false,
                            removed,
                        })
                    });
                    if let Some(mut item) = item {
                        item.removed |= removed;
                        items.push(item);
                    }
                }
                if let Some(token) = str_at(&page, &["nextPageToken"]) {
                    next = self.changes_url(token);
                    continue;
                }
                let token = str_at(&page, &["newStartPageToken"]).ok_or(MailError::NotFound)?;
                return Ok(FileChanges {
                    items,
                    cursor: serde_json::json!({ "link": self.changes_url(token) }),
                });
            }
            items.extend(self.items(&page).into_iter().filter(|i| !i.folder));
            if let Some(link) = str_at(&page, &["@odata.nextLink"]) {
                next = link.to_string();
                continue;
            }
            let link = str_at(&page, &["@odata.deltaLink"]).ok_or(MailError::NotFound)?;
            return Ok(FileChanges {
                items,
                cursor: serde_json::json!({ "link": link }),
            });
        }
        Ok(FileChanges {
            items,
            cursor: serde_json::json!({ "link": next }),
        })
    }
}

fn cap(text: String) -> String {
    match text.char_indices().nth(MAX_TEXT_CHARS) {
        Some((at, _)) => format!("{}\n[... cut at {MAX_TEXT_CHARS} characters]", &text[..at]),
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shared_onedrive_items_use_the_remote_id() {
        let item = onedrive_item(&json!({
            "id": "local", "name": "Plan.md", "webUrl": "https://x/Plan.md",
            "remoteItem": {"id": "remote-1", "file": {"mimeType": "text/markdown"}, "size": 10}
        }))
        .unwrap();
        assert_eq!(item.id, "onedrive:remote-1");
        assert_eq!(item.size, Some(10));
        assert!(is_text(&item.name, item.mime_type.as_deref()));
        assert!(!is_text("deck.pptx", Some("application/vnd.ms-powerpoint")));
    }

    #[test]
    fn google_docs_export_as_text() {
        assert_eq!(
            google_export("application/vnd.google-apps.document"),
            Some("text/plain")
        );
        assert_eq!(
            google_export("application/vnd.google-apps.spreadsheet"),
            Some("text/csv")
        );
        assert_eq!(google_export("application/pdf"), None);
        let item = drive_item(&json!({"id": "d1", "name": "Notes", "size": "42",
            "mimeType": "application/vnd.google-apps.document"}))
        .unwrap();
        assert_eq!(item.size, Some(42));
        assert_eq!(item.id, "google-drive:d1");
    }

    #[test]
    fn text_is_capped() {
        let long = "a".repeat(MAX_TEXT_CHARS + 10);
        assert!(cap(long).contains("cut at"));
    }
}
