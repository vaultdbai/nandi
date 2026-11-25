//! A browser's own door into the egress proxy: a loopback port that takes
//! CONNECT tunnels only, every one as one proxy identity.
//!
//! ```text
//! Chrome / Edge on the host ─▶ http://127.0.0.1:<door> (no sign-in asked)
//!   ─▶ the door adds the identity's token ─▶ the proxy's own tunnel check,
//!      record and refusal hook (`tunnel::handle_connect`)
//! ```
//!
//! Why not the proxy's own port with the token as the proxy password:
//! Chrome asks the proxy for its password with a 407 on every connection it
//! makes itself (the default search engine's preconnect, its sign-in and
//! update checks), and Playwright answers 407s only for the pages it drives.
//! Those questions stay open, and a page opening the same host (Google) then
//! waits behind them until Playwright gives up after 60 seconds. With no
//! 407 there is nothing to answer.
//!
//! The door lives as long as its [`Door`]: dropping it stops it taking
//! tunnels. Other routes (`/fetch`, the model proxies) aren't served here.

use std::convert::Infallible;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};

use super::{tunnel, ProxyContext, SERVER};

/// One browser's door; it closes when dropped
pub struct Door {
    port: u16,
    task: tokio::task::JoinHandle<()>,
}

impl Door {
    /// The proxy URL the browser is given
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for Door {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl std::fmt::Debug for Door {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Door").field("port", &self.port).finish()
    }
}

/// Opens a door on this process's proxy for the holder of `token`
pub fn open(token: &str) -> Result<Door, String> {
    let server = SERVER
        .get()
        .ok_or("Nandi's egress proxy isn't running, so the browser can't reach the web")?;
    open_on(&server.runtime, server.context.clone(), token)
}

/// [`open`] on `runtime`, with `context`'s rules
pub(super) fn open_on(
    runtime: &tokio::runtime::Handle,
    context: ProxyContext,
    token: &str,
) -> Result<Door, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .and_then(|listener| {
            listener.set_nonblocking(true)?;
            Ok(listener)
        })
        .map_err(|e| format!("Couldn't open the browser's way through Nandi's proxy: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Couldn't open the browser's way through Nandi's proxy: {e}"))?
        .port();
    let bearer = HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| "The browser's proxy token isn't a valid header".to_string())?;
    let task = runtime.spawn(async move {
        let listener = match tokio::net::TcpListener::from_std(listener) {
            Ok(listener) => listener,
            Err(e) => {
                log::error!("A browser's door into the egress proxy didn't open: {e}");
                return;
            }
        };
        let service = tower::service_fn(move |mut request: Request<Body>| {
            let context = context.clone();
            let bearer = bearer.clone();
            async move {
                if request.method() != Method::CONNECT {
                    return Ok::<_, Infallible>(StatusCode::METHOD_NOT_ALLOWED.into_response());
                }
                // Whatever the browser sent, the door's identity is the one
                request
                    .headers_mut()
                    .insert(header::PROXY_AUTHORIZATION, bearer);
                Ok::<Response, Infallible>(tunnel::handle_connect(context, request).await)
            }
        });
        if let Err(e) = axum::serve(
            listener,
            axum::ServiceExt::<Request<Body>>::into_make_service(service),
        )
        .await
        {
            log::warn!("A browser's door into the egress proxy closed: {e}");
        }
    });
    Ok(Door { port, task })
}

#[cfg(test)]
mod tests {
    use super::super::tests::{allow_hosts, context};
    use super::*;
    use crate::runtime::ProxyTokens;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpStream;

    /// An upstream that echoes what it receives
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

    /// Sends `request` to the door and reads the response head
    async fn send(door: &Door, request: &str) -> (TcpStream, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", door.port)).await.unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
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

    /// No 407 to answer: the door signs the tunnel in as its identity, and
    /// the proxy's rules still decide where it goes
    #[tokio::test]
    async fn a_door_tunnels_as_its_identity_with_no_sign_in_asked() {
        let tokens = ProxyTokens::default();
        let token = tokens.issue("browser:writer");
        let echo = start_echo().await;
        let door = open_on(
            &tokio::runtime::Handle::current(),
            context(tokens, allow_hosts(&[&format!("127.0.0.1:{echo}")])),
            &token,
        )
        .unwrap();
        assert!(door.url().starts_with("http://127.0.0.1:"));

        let target = format!("127.0.0.1:{echo}");
        let (mut stream, head) = send(
            &door,
            &format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n"),
        )
        .await;
        assert!(head.starts_with("HTTP/1.1 200"), "{head}");
        stream.write_all(b"ping").await.unwrap();
        let mut reply = [0u8; 4];
        stream.read_exact(&mut reply).await.unwrap();
        assert_eq!(&reply, b"ping");

        let (_, head) = send(
            &door,
            "CONNECT example.org:443 HTTP/1.1\r\nHost: example.org:443\r\n\r\n",
        )
        .await;
        assert!(
            head.starts_with("HTTP/1.1 403"),
            "not an allowed site: {head}"
        );
        let (_, head) = send(&door, "GET /fetch HTTP/1.1\r\nHost: x\r\n\r\n").await;
        assert!(head.starts_with("HTTP/1.1 405"), "tunnels only: {head}");

        let port = door.port;
        drop(door);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            TcpStream::connect(("127.0.0.1", port)).await.is_err(),
            "a dropped door takes nothing"
        );
    }

    /// A revoked token (the browser closed) opens nothing
    #[tokio::test]
    async fn a_door_with_a_revoked_token_tunnels_nowhere() {
        let tokens = ProxyTokens::default();
        let token = tokens.issue("browser:writer");
        let echo = start_echo().await;
        let door = open_on(
            &tokio::runtime::Handle::current(),
            context(tokens.clone(), allow_hosts(&[&format!("127.0.0.1:{echo}")])),
            &token,
        )
        .unwrap();
        tokens.revoke("browser:writer");
        let target = format!("127.0.0.1:{echo}");
        let (_, head) = send(
            &door,
            &format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n\r\n"),
        )
        .await;
        assert!(head.starts_with("HTTP/1.1 407"), "{head}");
    }
}
