//! Host-side LLM proxy for agent containers.
//!
//! `POST http://host.docker.internal:<proxy port>/llm` (11435 unless taken) with
//! `Authorization: Bearer $NANDI_LLM_PROXY_TOKEN` and an [`LlmRequest`] body
//! returns an [`LlmResponse`]. The host injects the provider API key, so keys
//! never enter containers.
//!
//! Served by the egress proxy (see the module docs in `egress`). Every
//! caller must present a per-agent token issued at container start; the
//! agent in the audit log comes from that token, not from the request.

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use futures_util::future::BoxFuture;
use serde_json::json;

use crate::llm::{self, CallSource, LlmRequest, LlmResponse};
use crate::runtime::ProxyTokens;
use crate::services::AppState;

const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Runs a completion on behalf of an agent: `(request, agent_id)`.
type CompleteFn = Arc<
    dyn Fn(LlmRequest, String) -> BoxFuture<'static, Result<LlmResponse, String>> + Send + Sync,
>;

#[derive(Clone)]
pub(super) struct ProxyState {
    tokens: ProxyTokens,
    complete: CompleteFn,
}

impl ProxyState {
    /// Completions go through the host's provider keys
    pub(super) fn new(state: &AppState) -> Self {
        let secrets = state.secrets().clone();
        let complete: CompleteFn = Arc::new(move |request, agent_id| {
            let secrets = secrets.clone();
            Box::pin(async move {
                llm::complete(&secrets, &request, Some(&agent_id), CallSource::Proxy).await
            })
        });
        Self {
            tokens: state.proxy_tokens().clone(),
            complete,
        }
    }
}

pub(super) fn router(state: ProxyState) -> Router {
    Router::new()
        .route("/llm", post(handle_llm))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

async fn handle_llm(
    State(state): State<ProxyState>,
    headers: HeaderMap,
    body: Result<Json<LlmRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    // Authenticate before looking at the body
    let Some(agent_id) = bearer_token(&headers).and_then(|t| state.tokens.agent_for(t)) else {
        return error(StatusCode::UNAUTHORIZED, "missing or invalid proxy token");
    };
    let Json(request) = match body {
        Ok(body) => body,
        Err(rejection) => return error(StatusCode::BAD_REQUEST, &rejection.body_text()),
    };
    if let Err(e) = llm::parse_model(&request.model) {
        return error(StatusCode::BAD_REQUEST, &e);
    }

    log::debug!("LLM proxy: agent '{agent_id}' → {}", request.model);
    match (state.complete)(request, agent_id).await {
        Ok(response) => Json(response).into_response(),
        Err(e) => error(StatusCode::BAD_GATEWAY, &e),
    }
}

pub(super) fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .filter(|token| !token.is_empty())
}

pub(super) fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use tower::ServiceExt;

    fn test_router(tokens: ProxyTokens) -> Router {
        let complete: CompleteFn = Arc::new(|request, agent_id| {
            Box::pin(async move {
                Ok(LlmResponse {
                    content: format!("{agent_id} via {}", request.model),
                    ..Default::default()
                })
            })
        });
        router(ProxyState { tokens, complete })
    }

    fn post_llm(token: Option<&str>, body: &str) -> Request<Body> {
        let mut request = Request::post("/llm").header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        request.body(Body::from(body.to_string())).unwrap()
    }

    const VALID_BODY: &str =
        r#"{"model":"ollama:llama3.2","messages":[{"role":"user","content":"hi"}]}"#;

    async fn json_body(response: Response) -> serde_json::Value {
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap()
    }

    #[tokio::test]
    async fn rejects_missing_and_unknown_tokens() {
        let tokens = ProxyTokens::default();
        tokens.issue("writer");

        for token in [None, Some("guess")] {
            let response = test_router(tokens.clone())
                .oneshot(post_llm(token, VALID_BODY))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "{token:?}");
        }
    }

    #[tokio::test]
    async fn attributes_request_to_token_owner() {
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");

        let response = test_router(tokens)
            .oneshot(post_llm(Some(&token), VALID_BODY))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            json_body(response).await["content"],
            "writer via ollama:llama3.2"
        );
    }

    #[tokio::test]
    async fn routes_deepseek_models() {
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");
        let body =
            r#"{"model":"deepseek:deepseek-flash","messages":[{"role":"user","content":"hi"}]}"#;

        let response = test_router(tokens)
            .oneshot(post_llm(Some(&token), body))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            json_body(response).await["content"],
            "writer via deepseek:deepseek-flash"
        );
    }

    #[tokio::test]
    async fn rejects_malformed_body_and_unknown_provider() {
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");

        let response = test_router(tokens.clone())
            .oneshot(post_llm(Some(&token), "{not json"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = r#"{"model":"gemini:pro","messages":[]}"#;
        let response = test_router(tokens)
            .oneshot(post_llm(Some(&token), body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(json_body(response).await["error"]
            .as_str()
            .unwrap()
            .contains("Unknown model provider"));
    }
}
