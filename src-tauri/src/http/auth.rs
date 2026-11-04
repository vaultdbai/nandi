//! Web-port authentication. Every request needs the token from
//! `~/.nandi/web_token`, even on localhost:
//!
//! - `Authorization: Bearer <token>` (scripts), or
//! - the `nandi_token` cookie (browsers; also covers the WebSocket, which
//!   can't send custom headers), or
//! - `?token=<token>` on first load. For a page request this is exchanged
//!   for an HttpOnly, SameSite=Strict cookie and a redirect that strips the
//!   token from the URL (and so from history and logs).

use std::path::Path;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use serde_json::json;

const COOKIE_NAME: &str = "nandi_token";

/// The shared secret for the web port. Cheap to clone.
#[derive(Clone)]
pub struct WebToken(Arc<str>);

impl WebToken {
    #[cfg(test)]
    pub fn new(value: &str) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Reads the token, creating it (owner-only permissions) on first launch.
    pub fn load_or_create(path: &Path) -> Result<Self, String> {
        if let Ok(existing) = std::fs::read_to_string(path) {
            let existing = existing.trim();
            if existing.len() >= 32 {
                return Ok(Self(existing.into()));
            }
        }
        let token = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, &token).map_err(|e| format!("Failed to write web token: {e}"))?;
        restrict_to_owner(path)?;
        log::info!("Created web access token at {}", path.display());
        Ok(Self(token.into()))
    }

    /// Constant-time comparison.
    fn matches(&self, candidate: &str) -> bool {
        let (a, b) = (self.0.as_bytes(), candidate.as_bytes());
        a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
    }
}

#[cfg(unix)]
fn restrict_to_owner(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("Failed to restrict web token permissions: {e}"))
}

/// On Windows the file inherits the user profile's ACL (owner + admins).
#[cfg(not(unix))]
fn restrict_to_owner(_path: &Path) -> Result<(), String> {
    Ok(())
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// `token` from the query string, and the query without it.
fn split_query_token(uri: &Uri) -> (Option<String>, String) {
    let mut token = None;
    let rest: Vec<&str> = uri
        .query()
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty())
        .filter(|pair| match pair.strip_prefix("token=") {
            Some(value) => {
                token = Some(value.to_string());
                false
            }
            None => true,
        })
        .collect();
    (token, rest.join("&"))
}

fn is_api(path: &str) -> bool {
    path.starts_with("/api/") || path == "/ws"
}

/// Webhook and device receivers: called by external systems, which hold a
/// trigger's secret (checked by the route), not the web token.
fn is_trigger_receiver(path: &str) -> bool {
    path.starts_with("/api/triggers/webhook/") || path.starts_with("/api/triggers/device/")
}

pub async fn require_token(
    State(token): State<WebToken>,
    request: Request,
    next: Next,
) -> Response {
    if is_trigger_receiver(request.uri().path()) {
        return next.run(request).await;
    }
    let headers = request.headers();
    if bearer(headers).is_some_and(|t| token.matches(t)) || cookie_matches(headers, &token) {
        return next.run(request).await;
    }

    let (query_token, rest) = split_query_token(request.uri());
    if query_token.as_deref().is_some_and(|t| token.matches(t)) {
        let path = request.uri().path().to_string();
        if is_api(&path) {
            return next.run(request).await;
        }
        // Page load: swap the token for a cookie and drop it from the URL
        let location = if rest.is_empty() {
            path
        } else {
            format!("{path}?{rest}")
        };
        let mut response = Redirect::to(&location).into_response();
        let cookie = format!(
            "{COOKIE_NAME}={}; HttpOnly; SameSite=Strict; Path=/",
            token.as_str()
        );
        if let Ok(value) = HeaderValue::from_str(&cookie) {
            response.headers_mut().insert(header::SET_COOKIE, value);
        }
        return response;
    }

    if is_api(request.uri().path()) {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "unauthorized" })),
        )
            .into_response()
    } else {
        (
            StatusCode::UNAUTHORIZED,
            "Nandi web access needs a token. Open the link printed at startup, or add ?token=<token> (see ~/.nandi/web_token).",
        )
            .into_response()
    }
}

fn cookie_matches(headers: &HeaderMap, token: &WebToken) -> bool {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().strip_prefix(COOKIE_NAME)?.strip_prefix('='))
        .any(|value| token.matches(value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_tokens_exactly() {
        let token = WebToken::new("abc123");
        assert!(token.matches("abc123"));
        assert!(!token.matches("abc12"));
        assert!(!token.matches("abc124"));
    }

    #[test]
    fn strips_only_the_token_from_the_query() {
        let uri: Uri = "/studio?x=1&token=secret&y=2".parse().unwrap();
        assert_eq!(
            split_query_token(&uri),
            (Some("secret".into()), "x=1&y=2".into())
        );
        let uri: Uri = "/".parse().unwrap();
        assert_eq!(split_query_token(&uri), (None, String::new()));
    }

    #[test]
    fn only_trigger_receivers_skip_the_token() {
        assert!(is_trigger_receiver("/api/triggers/webhook/abc"));
        assert!(is_trigger_receiver("/api/triggers/device/printer-01"));
        assert!(!is_trigger_receiver("/api/triggers"));
        assert!(!is_trigger_receiver("/api/triggers/abc/test"));
        assert!(!is_trigger_receiver("/api/triggers/webhook"));
    }

    #[test]
    fn creates_and_reuses_the_token_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("web_token");
        let first = WebToken::load_or_create(&path).unwrap();
        let second = WebToken::load_or_create(&path).unwrap();
        assert_eq!(first.as_str(), second.as_str());
        assert_eq!(first.as_str().len(), 64);
    }
}
