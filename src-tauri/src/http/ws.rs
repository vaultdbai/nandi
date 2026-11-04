//! `GET /ws`: every event from the bus, as JSON text frames
//! `{"event": "agent://token", "payload": {…}}`. Clients filter by name.
//!
//! If a client falls too far behind, it gets `{"event": "nandi://resync"}`
//! and should reload its snapshots (the DB has everything it missed).

use axum::extract::ws::{Message, Utf8Bytes, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::Response;
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;

use crate::services::AppState;

pub const RESYNC_EVENT: &str = "nandi://resync";

pub async fn handler(State(state): State<AppState>, upgrade: WebSocketUpgrade) -> Response {
    let events = state.broadcast().subscribe();
    upgrade.on_upgrade(move |socket| forward(socket, events))
}

async fn forward(
    mut socket: WebSocket,
    mut events: tokio::sync::broadcast::Receiver<crate::services::events::WireEvent>,
) {
    loop {
        tokio::select! {
            event = events.recv() => {
                let text = match event {
                    Ok(event) => serde_json::to_string(&event).unwrap_or_default(),
                    Err(RecvError::Lagged(missed)) => {
                        log::warn!("WebSocket client lagged by {missed} events; asking it to resync");
                        json!({ "event": RESYNC_EVENT, "payload": { "missed": missed } }).to_string()
                    }
                    Err(RecvError::Closed) => break,
                };
                if socket.send(Message::Text(Utf8Bytes::from(text))).await.is_err() {
                    break; // client went away
                }
            }
            incoming = socket.recv() => match incoming {
                // Clients don't send anything meaningful; just notice closes
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                Some(Ok(_)) => {}
            },
        }
    }
}
