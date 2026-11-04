//! The one "not connected" answer of a person-to-person action.
//!
//! Asking or assigning someone else, reassigning, handing over and finding
//! owners need the relay. When no relay is configured, or it isn't
//! connected right now, those actions answer [`RelayUnavailable`] instead of
//! an ad-hoc error. Commands return errors as strings, so the error crosses
//! the bridge as text that starts with [`RELAY_UNAVAILABLE`]; the constant is
//! exported to TypeScript (`bindings.ts`) and the UI shows "Connect to your
//! company's relay" for it instead of an error.
//!
//! Local-only actions never return it: everything that doesn't involve
//! another person works without a relay.

use std::sync::Arc;

use super::RelayClient;

/// The stable code a [`RelayUnavailable`] error starts with
pub const RELAY_UNAVAILABLE: &str = "relay_unavailable";

/// A person-to-person action needs the relay, and none is connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RelayUnavailable;

impl RelayUnavailable {
    /// Whether `error` (a command's error text) is this one
    pub fn is(error: &str) -> bool {
        error.starts_with(RELAY_UNAVAILABLE)
    }
}

impl std::fmt::Display for RelayUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{RELAY_UNAVAILABLE}: this needs your company's relay, and Nandi isn't connected to one"
        )
    }
}

impl std::error::Error for RelayUnavailable {}

impl From<RelayUnavailable> for String {
    fn from(error: RelayUnavailable) -> Self {
        error.to_string()
    }
}

/// The relay client, when it is connected right now
pub fn connected(
    client: Option<&Arc<dyn RelayClient>>,
) -> Result<&Arc<dyn RelayClient>, RelayUnavailable> {
    client
        .filter(|client| client.status().connected)
        .ok_or(RelayUnavailable)
}

/// The relay client, when a relay is configured (connected, or connecting
/// in the background). For work that may wait for the relay, like a
/// request queued until its person is back.
pub fn configured(
    client: Option<&Arc<dyn RelayClient>>,
) -> Result<&Arc<dyn RelayClient>, RelayUnavailable> {
    client
        .filter(|client| client.status().relay_url.is_some())
        .ok_or(RelayUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::relay::client::WsRelayClient;

    #[test]
    fn the_error_text_starts_with_the_stable_code() {
        let text = String::from(RelayUnavailable);
        assert!(text.starts_with("relay_unavailable: "));
        assert!(RelayUnavailable::is(&text));
        assert!(!RelayUnavailable::is("Couldn't reach the relay"));
    }

    #[test]
    fn a_client_with_no_relay_is_neither_connected_nor_configured() {
        let client: Arc<dyn RelayClient> = Arc::new(WsRelayClient::new(None, "me"));
        assert_eq!(connected(Some(&client)).err(), Some(RelayUnavailable));
        assert_eq!(configured(Some(&client)).err(), Some(RelayUnavailable));
        assert_eq!(connected(None).err(), Some(RelayUnavailable));
    }

    #[tokio::test]
    async fn an_unreachable_relay_is_configured_but_not_connected() {
        let client: Arc<dyn RelayClient> = Arc::new(WsRelayClient::new(None, "me"));
        // Port 1: nothing listens; the client keeps retrying in the background
        let _ = client.connect("ws://127.0.0.1:1/relay", "me").await;
        assert!(configured(Some(&client)).is_ok());
        assert_eq!(connected(Some(&client)).err(), Some(RelayUnavailable));
        let _ = client.connect("", "me").await;
    }
}
