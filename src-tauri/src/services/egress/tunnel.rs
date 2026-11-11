//! `CONNECT host:port` tunnels, so tools that honor `HTTPS_PROXY` work
//! unchanged. The agent authenticates with its proxy token as the Basic
//! password (`http://nandi:<token>@host.docker.internal:11435`), and the
//! target must pass [`Capabilities::allows_tunnel`].
//!
//! [`Capabilities::allows_tunnel`]: super::capabilities::Capabilities::allows_tunnel

use std::time::Instant;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;

use super::capabilities::Capabilities;
use super::{address, record, EgressKind, EgressLogEntry, ProxyContext};
use crate::audit;

const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

pub(super) async fn handle_connect(context: ProxyContext, mut request: Request<Body>) -> Response {
    let started = Instant::now();
    let Some(authority) = request.uri().authority().cloned() else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let host = authority
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    let port = authority.port_u16().unwrap_or(443);

    let Some(agent_id) =
        proxy_token(request.headers()).and_then(|token| context.tokens.agent_for(&token))
    else {
        return Response::builder()
            .status(StatusCode::PROXY_AUTHENTICATION_REQUIRED)
            .header(header::PROXY_AUTHENTICATE, "Basic realm=\"nandi\"")
            .body(Body::empty())
            .unwrap_or_default();
    };

    let target = format!("{host}:{port}");
    let entry = |allowed: bool, status: StatusCode, error: Option<String>| EgressLogEntry {
        ts: audit::now_ms() as f64,
        agent_id: agent_id.clone(),
        kind: EgressKind::Tunnel,
        target: target.clone(),
        allowed,
        status: Some(status.as_u16()),
        duration_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
        error,
    };

    let capabilities = (context.capabilities)(&agent_id).ok();
    let allowed = capabilities
        .as_ref()
        .is_some_and(|caps| caps.allows_tunnel(&host, port));
    if !allowed {
        record(entry(
            false,
            StatusCode::FORBIDDEN,
            Some("not in network.allow".to_string()),
        ))
        .await;
        return StatusCode::FORBIDDEN.into_response();
    }

    // Resolved once and checked: the tunnel goes to these addresses only
    let named = |ip| {
        capabilities
            .as_ref()
            .is_some_and(|caps| caps.names_address(ip, Capabilities::tunnel_port(port)))
    };
    let addrs = match address::reachable(&host, port, named).await {
        Ok(addrs) => addrs,
        Err(e) => {
            record(entry(false, StatusCode::FORBIDDEN, Some(e))).await;
            return StatusCode::FORBIDDEN.into_response();
        }
    };

    // Connect first, so a dead upstream is a 502 rather than a dropped tunnel
    let upstream = match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(&addrs[..])).await
    {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            record(entry(true, StatusCode::BAD_GATEWAY, Some(e.to_string()))).await;
            return StatusCode::BAD_GATEWAY.into_response();
        }
        Err(_) => {
            record(entry(
                true,
                StatusCode::GATEWAY_TIMEOUT,
                Some("connect timed out".to_string()),
            ))
            .await;
            return StatusCode::GATEWAY_TIMEOUT.into_response();
        }
    };
    record(entry(true, StatusCode::OK, None)).await;

    let upgrade = hyper::upgrade::on(&mut request);
    tokio::spawn(async move {
        let mut upstream = upstream;
        match upgrade.await {
            Ok(upgraded) => {
                let mut client = TokioIo::new(upgraded);
                if let Err(e) = tokio::io::copy_bidirectional(&mut client, &mut upstream).await {
                    log::debug!("Tunnel to {target} closed: {e}");
                }
            }
            Err(e) => log::warn!("Tunnel to {target} failed to upgrade: {e}"),
        }
    });
    StatusCode::OK.into_response()
}

/// The token from `Proxy-Authorization: Basic base64(user:token)` (or
/// `Bearer token`).
fn proxy_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::PROXY_AUTHORIZATION)?.to_str().ok()?;
    if let Some(token) = value.strip_prefix("Bearer ") {
        return Some(token.to_string()).filter(|t| !t.is_empty());
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(value.strip_prefix("Basic ")?)
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (_, token) = decoded.split_once(':')?;
    (!token.is_empty()).then(|| token.to_string())
}

#[cfg(test)]
mod tests {
    use super::super::tests::{allow_hosts, context};
    use super::*;
    use crate::runtime::ProxyTokens;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Runs the real proxy server on a random port.
    async fn start_proxy(tokens: ProxyTokens, hosts: &[&str]) -> u16 {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let context = context(tokens, allow_hosts(hosts));
        tokio::spawn(super::super::serve(listener, axum::Router::new(), context));
        port
    }

    /// An upstream that echoes what it receives.
    async fn start_echo() -> u16 {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let (mut read, mut write) = socket.split();
                    let _ = tokio::io::copy(&mut read, &mut write).await;
                });
            }
        });
        port
    }

    async fn connect(proxy_port: u16, target: &str, token: Option<&str>) -> (TcpStream, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", proxy_port)).await.unwrap();
        let mut request = format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n");
        if let Some(token) = token {
            let basic = base64::engine::general_purpose::STANDARD.encode(format!("nandi:{token}"));
            request.push_str(&format!("Proxy-Authorization: Basic {basic}\r\n"));
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();

        // Read just the response head
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if stream.read(&mut byte).await.unwrap() == 0 {
                break;
            }
            head.push(byte[0]);
        }
        (stream, String::from_utf8(head).unwrap())
    }

    #[tokio::test]
    async fn tunnels_to_allowed_hosts_only() {
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");
        let echo = start_echo().await;
        let proxy = start_proxy(tokens, &[&format!("127.0.0.1:{echo}")]).await;

        let (_, head) = connect(proxy, &format!("127.0.0.1:{echo}"), None).await;
        assert!(head.starts_with("HTTP/1.1 407"), "{head}");
        let (_, head) = connect(proxy, &format!("127.0.0.1:{echo}"), Some("guess")).await;
        assert!(head.starts_with("HTTP/1.1 407"), "{head}");

        let (_, head) = connect(proxy, "127.0.0.1:1", Some(&token)).await;
        assert!(head.starts_with("HTTP/1.1 403"), "{head}");

        let (mut stream, head) = connect(proxy, &format!("127.0.0.1:{echo}"), Some(&token)).await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        stream.write_all(b"ping").await.unwrap();
        let mut reply = [0u8; 4];
        stream.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"ping");
    }

    #[tokio::test]
    async fn a_tunnel_to_a_name_that_resolves_to_the_host_is_refused() {
        let tokens = ProxyTokens::default();
        let token = tokens.issue("writer");
        let echo = start_echo().await;
        let proxy = start_proxy(tokens, &[&format!("localhost:{echo}"), "*"]).await;

        for target in [
            format!("localhost:{echo}"),
            "169.254.169.254:443".to_string(),
        ] {
            let (_, head) = connect(proxy, &target, Some(&token)).await;
            assert!(head.starts_with("HTTP/1.1 403"), "{target}: {head}");
        }
    }

    #[test]
    fn reads_proxy_credentials() {
        let mut headers = HeaderMap::new();
        assert!(proxy_token(&headers).is_none());
        let basic = base64::engine::general_purpose::STANDARD.encode("nandi:abc");
        headers.insert(
            header::PROXY_AUTHORIZATION,
            format!("Basic {basic}").parse().unwrap(),
        );
        assert_eq!(proxy_token(&headers).as_deref(), Some("abc"));
        headers.insert(header::PROXY_AUTHORIZATION, "Bearer xyz".parse().unwrap());
        assert_eq!(proxy_token(&headers).as_deref(), Some("xyz"));
    }
}
