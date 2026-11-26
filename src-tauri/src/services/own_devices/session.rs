//! A paired device's ask session: the PIN once, then asks without it
//! until the device goes idle for [`IDLE_MS`] (10 minutes by default).
//! Approvals and declines still carry the PIN every time.
//!
//! - A right PIN, with any call, opens or renews the session.
//! - An ask within the session renews it; reading the Inbox or a chat
//!   doesn't, so a phone left polling still locks.
//! - A wrong PIN, an unlock on the desktop or a revoke ends it.
//!
//! Sessions live in this process's memory only: when Nandi restarts,
//! every device needs its PIN again.

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

/// How long a device may go without asking before its PIN is needed again
pub const IDLE_MS: i64 = 10 * 60 * 1000;

/// When each device last proved its PIN or asked, in epoch ms
#[derive(Debug, Default)]
pub struct Sessions {
    last: Mutex<HashMap<String, i64>>,
}

impl Sessions {
    /// The sessions of this Nandi (one per process)
    pub fn shared() -> Arc<Self> {
        static SHARED: LazyLock<Arc<Sessions>> = LazyLock::new(Arc::default);
        SHARED.clone()
    }

    fn map(&self) -> std::sync::MutexGuard<'_, HashMap<String, i64>> {
        self.last.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether `device_id`'s session is open at `now`, after `idle_ms`
    /// without a renewal
    pub fn is_open(&self, device_id: &str, now: i64, idle_ms: i64) -> bool {
        self.map()
            .get(device_id)
            .is_some_and(|last| now >= *last && now - *last < idle_ms)
    }

    /// Opens or renews `device_id`'s session at `now`
    pub fn renew(&self, device_id: &str, now: i64) {
        self.map().insert(device_id.to_string(), now);
    }

    /// Ends `device_id`'s session: its next ask needs the PIN
    pub fn end(&self, device_id: &str) {
        self.map().remove(device_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_opens_lapses_after_idle_and_ends() {
        let sessions = Sessions::default();
        assert!(!sessions.is_open("d1", 1_000, IDLE_MS));
        sessions.renew("d1", 1_000);
        assert!(sessions.is_open("d1", 1_000 + IDLE_MS - 1, IDLE_MS));
        assert!(!sessions.is_open("d1", 1_000 + IDLE_MS, IDLE_MS));
        // Renewing moves the idle point
        sessions.renew("d1", 1_000 + IDLE_MS);
        assert!(sessions.is_open("d1", 1_000 + IDLE_MS + 5, IDLE_MS));
        // Another device's session is its own
        assert!(!sessions.is_open("d2", 1_000, IDLE_MS));
        sessions.end("d1");
        assert!(!sessions.is_open("d1", 1_000 + IDLE_MS + 5, IDLE_MS));
        // A clock that went back doesn't count as inside the session
        sessions.renew("d1", 5_000);
        assert!(!sessions.is_open("d1", 4_000, IDLE_MS));
    }
}
