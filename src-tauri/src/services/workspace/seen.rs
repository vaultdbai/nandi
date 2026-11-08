//! Calendar events already handed to the Chief of Staff, so a change feed
//! that starts over (the weekly window move, an expired cursor, a feed that
//! ended without a token) sends only what is new or changed.
//!
//! Kept per calendar in `local_settings` `workspace.<id>.seen`: event ID to a
//! fingerprint of what was sent. A full read replaces it, so it never holds
//! more than one window.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::calendar::Event;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Seen(BTreeMap<String, String>);

impl Seen {
    fn fingerprint(event: &Event) -> String {
        let json = serde_json::to_vec(event).unwrap_or_default();
        format!("{:x}", Sha256::digest(json))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether `event` was handed over before, exactly as it is now
    pub fn knows(&self, event: &Event) -> bool {
        self.0
            .get(&event.id)
            .is_some_and(|fp| *fp == Self::fingerprint(event))
    }

    /// Records what a check read: a full read (`restarted`) replaces the
    /// record, a change feed adds to it
    pub fn record(&mut self, events: &[Event], restarted: bool) {
        if restarted {
            self.0.clear();
        }
        for event in events {
            self.0.insert(event.id.clone(), Self::fingerprint(event));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(id: &str, title: &str) -> Event {
        Event {
            id: id.into(),
            title: title.into(),
            start: "2026-11-09T10:00:00Z".into(),
            end: "2026-11-09T11:00:00Z".into(),
            all_day: false,
            organizer: None,
            attendees: Vec::new(),
            attendee_count: 0,
            time_off: false,
            cancelled: false,
            busy: true,
            location: None,
            link: None,
        }
    }

    #[test]
    fn knows_only_unchanged_events() {
        let mut seen = Seen::default();
        assert!(seen.is_empty());
        seen.record(&[event("g:1", "Standup")], true);
        assert!(seen.knows(&event("g:1", "Standup")));
        assert!(!seen.knows(&event("g:1", "Standup moved")));
        assert!(!seen.knows(&event("g:2", "Review")));
    }

    #[test]
    fn a_full_read_replaces_and_changes_add() {
        let mut seen = Seen::default();
        seen.record(&[event("g:1", "Standup")], true);
        seen.record(&[event("g:2", "Review")], false);
        assert!(seen.knows(&event("g:1", "Standup")));
        assert!(seen.knows(&event("g:2", "Review")));
        seen.record(&[event("g:2", "Review")], true);
        assert!(!seen.knows(&event("g:1", "Standup")));
        assert!(seen.knows(&event("g:2", "Review")));
    }
}
