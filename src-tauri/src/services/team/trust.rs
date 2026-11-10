//! Trust toward other members (wave final, Track E): trusted peers (also
//! `federation.trusted_peers` in NANDI.md), paused delegations (our gateway
//! rejects their tasks and we stop routing to them), and blocked agents
//! (filtered from listings and routing in both directions).
//!
//! `is_paused` and `is_blocked` are the checks the gateway (inbound) and
//! delegation (outbound) call; the shared listings already apply them.
//!
//! **Everyone on the company's relay is trusted**: a relay that needs a secret only lets in instances the company
//! enrolled, so when ours registered with a token, every member it lists
//! is trusted ([`is_trusted`]) unless the owner turns that off
//! ([`set_trusts_relay_members`]) and trusts by hand. Pause and Block still
//! apply on top, and assigned work still waits for its receiver's approval.
//! On an open relay (no token), trust stays manual.

use std::collections::HashSet;
use std::path::Path;

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use specta::Type;

use crate::db::{now_ms, Db};
use crate::model::Timestamp;
use crate::services::domain::agent_card::OwnerRef;
use crate::services::events::{emit, NandiEvent};
use crate::services::gateway::manifest;
use crate::services::repos::fs_atomic;
use crate::services::{network, AppState};

/// `trust://relationship_updated`
pub const TRUST_RELATIONSHIP_UPDATED: &str = "trust://relationship_updated";

/// A `trust_relationships` row plus that member's `blocked_members` rows
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrustRelationship {
    pub instance_id: String,
    pub added_as_trusted_peer: bool,
    pub delegations_paused: bool,
    /// Agent IDs of theirs we blocked
    pub blocked_agents: Vec<String>,
    pub last_updated: Option<Timestamp>,
}

impl NandiEvent for TrustRelationship {
    const NAME: &'static str = TRUST_RELATIONSHIP_UPDATED;
}

/// `None` fields keep their value
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrustUpdate {
    pub added_as_trusted_peer: Option<bool>,
    pub delegations_paused: Option<bool>,
}

/// The trust service. Stateless: everything lives in SQLite (and NANDI.md
/// for trusted peers); it exists so `AppState` reports trust as available.
pub struct TrustService {
    _private: (),
}

/// Never blocks
pub fn init(_state: &AppState) -> Option<std::sync::Arc<TrustService>> {
    Some(std::sync::Arc::new(TrustService { _private: () }))
}

// ---------------------------------------------------------------------------
// Checks (for the gateway, delegation, and the shared listings)
// ---------------------------------------------------------------------------

/// The owner marked `instance_id` a trusted peer (Network → Trust). A
/// database error reads as not trusted.
pub fn is_trusted_peer(db: &Db, instance_id: &str) -> bool {
    get_relationship(db, instance_id).is_ok_and(|r| r.added_as_trusted_peer)
}

/// `local_settings` key: trust everyone on the company's relay (default on)
pub const RELAY_TRUST_KEY: &str = "trust.relayMembers";

/// Trusted: marked by hand, or a member of the company's relay while that
/// is on (see the module docs)
pub fn is_trusted(db: &Db, instance_id: &str) -> bool {
    is_trusted_peer(db, instance_id) || trusted_by_relay(db, instance_id)
}

/// `instance_id` is on our relay, the relay needs a token (ours has one),
/// and the owner trusts its members
pub fn trusted_by_relay(db: &Db, instance_id: &str) -> bool {
    trusts_relay_members(db)
        && on_a_secured_relay(db)
        && network::known_members(db)
            .iter()
            .any(|member| member.instance_id == instance_id)
}

/// Whether our relay connection carries an instance token (from
/// `NANDI_RELAY_URL` or the saved one): the relay needs a secret, so only
/// instances the company enrolled are on it. Not a public relay: anyone
/// who signs in is on that one, so trust stays one by one there.
pub fn on_a_secured_relay(db: &Db) -> bool {
    crate::services::relay::instance_token(db).is_some() && !on_a_public_relay(db)
}

/// Whether our relay token is from signing in to a public relay
pub fn on_a_public_relay(db: &Db) -> bool {
    crate::services::relay::public::on_public_relay(db)
}

/// "Trust everyone on my company's relay" (on unless the owner turned it off)
pub fn trusts_relay_members(db: &Db) -> bool {
    db.conn()
        .ok()
        .and_then(|conn| {
            crate::db::get_setting(&conn, RELAY_TRUST_KEY)
                .ok()
                .flatten()
        })
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

/// Turns relay trust on or off
pub fn set_trusts_relay_members(db: &Db, on: bool) -> Result<(), String> {
    let conn = db.conn()?;
    conn.execute(
        "INSERT INTO local_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![RELAY_TRUST_KEY, serde_json::json!(on).to_string()],
    )
    .map_err(|e| format!("Database error: {e}"))?;
    Ok(())
}

/// What the Connection view shows about relay trust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RelayTrust {
    /// The owner's choice
    pub enabled: bool,
    /// Our relay needs a token, so the choice applies
    pub secured: bool,
    /// Our relay is a public one: anyone can sign in, so the choice
    /// doesn't apply
    #[serde(default)]
    pub public: bool,
}

pub fn relay_trust(db: &Db) -> RelayTrust {
    RelayTrust {
        enabled: trusts_relay_members(db),
        secured: on_a_secured_relay(db),
        public: on_a_public_relay(db),
    }
}

/// We paused delegations from (and to) `instance_id`. A database error
/// reads as not paused (logged).
pub fn is_paused(db: &Db, instance_id: &str) -> bool {
    let paused = db.conn().and_then(|conn| {
        conn.query_row(
            "SELECT delegations_paused FROM trust_relationships WHERE instance_id = ?1",
            [instance_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| format!("Database error: {e}"))
    });
    match paused {
        Ok(paused) => paused.unwrap_or(0) != 0,
        Err(e) => {
            log::warn!("Trust check failed: {e}");
            false
        }
    }
}

/// The `blocked_members` agent ID that blocks every agent of an instance
/// (wave EF): its tasks are rejected on arrival and nothing is routed to it
pub const ALL_AGENTS: &str = "*";

/// We blocked `agent_id` (an agent ID, or its `<instance>/<agent>` card
/// ID) of `instance_id`, or the whole instance. A database error reads as
/// not blocked (logged).
pub fn is_blocked(db: &Db, instance_id: &str, agent_id: &str) -> bool {
    let prefix = format!("{instance_id}/");
    let agent_id = agent_id.strip_prefix(&prefix).unwrap_or(agent_id);
    let blocked = db.conn().and_then(|conn| {
        conn.query_row(
            "SELECT 1 FROM blocked_members
              WHERE instance_id = ?1 AND agent_id IN (?2, '*') LIMIT 1",
            params![instance_id, agent_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|e| format!("Database error: {e}"))
    });
    match blocked {
        Ok(found) => found.is_some(),
        Err(e) => {
            log::warn!("Trust check failed: {e}");
            false
        }
    }
}

/// We blocked every agent of `instance_id`
pub fn is_instance_blocked(db: &Db, instance_id: &str) -> bool {
    is_blocked(db, instance_id, ALL_AGENTS)
}

/// Every paused instance and every blocked `(instance, agent)`, for
/// filtering a whole listing with two queries
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrustFilter {
    pub paused: HashSet<String>,
    pub blocked: HashSet<(String, String)>,
}

impl TrustFilter {
    pub fn load(db: &Db) -> Result<Self, String> {
        let conn = db.conn()?;
        let mut filter = Self::default();
        let mut stmt = conn
            .prepare("SELECT instance_id FROM trust_relationships WHERE delegations_paused != 0")
            .map_err(|e| format!("Database error: {e}"))?;
        filter.paused = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| format!("Database error: {e}"))?
            .collect::<rusqlite::Result<_>>()
            .map_err(|e| format!("Database error: {e}"))?;
        let mut stmt = conn
            .prepare("SELECT instance_id, agent_id FROM blocked_members")
            .map_err(|e| format!("Database error: {e}"))?;
        filter.blocked = stmt
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|e| format!("Database error: {e}"))?
            .collect::<rusqlite::Result<_>>()
            .map_err(|e| format!("Database error: {e}"))?;
        Ok(filter)
    }

    pub fn is_paused(&self, instance_id: &str) -> bool {
        self.paused.contains(instance_id)
    }

    pub fn is_blocked(&self, instance_id: &str, agent_id: &str) -> bool {
        [agent_id, ALL_AGENTS].iter().any(|agent| {
            self.blocked
                .contains(&(instance_id.to_string(), agent.to_string()))
        })
    }
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/// `instance_id`'s relationship (all defaults when we never set one)
pub fn get_relationship(db: &Db, instance_id: &str) -> Result<TrustRelationship, String> {
    let conn = db.conn()?;
    let row: Option<(i64, i64, i64)> = conn
        .query_row(
            "SELECT added_as_trusted_peer, delegations_paused, last_updated
               FROM trust_relationships WHERE instance_id = ?1",
            [instance_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| format!("Database error: {e}"))?;
    let mut stmt = conn
        .prepare(
            "SELECT agent_id, blocked_at FROM blocked_members
              WHERE instance_id = ?1 ORDER BY agent_id",
        )
        .map_err(|e| format!("Database error: {e}"))?;
    let blocked: Vec<(String, i64)> = stmt
        .query_map([instance_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| format!("Database error: {e}"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| format!("Database error: {e}"))?;
    let last_blocked = blocked.iter().map(|(_, at)| *at).max();
    let last_updated = match (row.map(|r| r.2), last_blocked) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    };
    Ok(TrustRelationship {
        instance_id: instance_id.to_string(),
        added_as_trusted_peer: row.is_some_and(|r| r.0 != 0),
        delegations_paused: row.is_some_and(|r| r.1 != 0),
        blocked_agents: blocked.into_iter().map(|(agent, _)| agent).collect(),
        last_updated: last_updated.map(|ms| ms as Timestamp),
    })
}

/// Upserts the row; `None` fields keep their value
pub fn update_relationship(
    db: &Db,
    instance_id: &str,
    update: &TrustUpdate,
) -> Result<TrustRelationship, String> {
    let current = get_relationship(db, instance_id)?;
    let trusted = update
        .added_as_trusted_peer
        .unwrap_or(current.added_as_trusted_peer);
    let paused = update
        .delegations_paused
        .unwrap_or(current.delegations_paused);
    db.conn()?
        .execute(
            "INSERT INTO trust_relationships
               (instance_id, added_as_trusted_peer, delegations_paused, last_updated)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(instance_id) DO UPDATE SET
               added_as_trusted_peer = excluded.added_as_trusted_peer,
               delegations_paused = excluded.delegations_paused,
               last_updated = excluded.last_updated",
            params![instance_id, trusted, paused, now_ms()],
        )
        .map_err(|e| format!("Database error: {e}"))?;
    get_relationship(db, instance_id)
}

pub fn set_blocked(
    db: &Db,
    instance_id: &str,
    agent_id: &str,
    blocked: bool,
) -> Result<TrustRelationship, String> {
    {
        let conn = db.conn()?;
        if blocked {
            conn.execute(
                "INSERT OR IGNORE INTO blocked_members (instance_id, agent_id, blocked_at)
                 VALUES (?1, ?2, ?3)",
                params![instance_id, agent_id, now_ms()],
            )
        } else {
            conn.execute(
                "DELETE FROM blocked_members WHERE instance_id = ?1 AND agent_id = ?2",
                params![instance_id, agent_id],
            )
        }
        .map_err(|e| format!("Database error: {e}"))?;
    }
    // Stamps the change: an unblock removes the row `last_updated` read
    update_relationship(
        db,
        instance_id,
        &TrustUpdate {
            added_as_trusted_peer: None,
            delegations_paused: None,
        },
    )
}

// ---------------------------------------------------------------------------
// NANDI.md `federation.trusted_peers`
// ---------------------------------------------------------------------------

/// Adds (`trusted`) or removes `peer` in the frontmatter's
/// `federation.trusted_peers` (`{github_handle, display_name}` entries,
/// matched by handle). Returns the new text, or `None` when nothing
/// changed. Only that list is touched: every other key keeps its value and
/// order, and the Markdown body is kept byte for byte (the frontmatter is
/// re-serialized, so YAML comments and quoting style there are not kept).
pub fn edit_trusted_peers(
    text: &str,
    peer: &OwnerRef,
    trusted: bool,
) -> Result<Option<String>, String> {
    let (yaml, tail) = split_frontmatter(text).ok_or("NANDI.md has no frontmatter")?;
    let mut root: Value =
        serde_yaml::from_str(yaml).map_err(|e| format!("NANDI.md frontmatter is invalid: {e}"))?;
    if root.is_null() {
        root = Value::Mapping(Mapping::new());
    }
    let peers = child(&mut root, "federation", || Value::Mapping(Mapping::new()))?;
    let peers = child(peers, "trusted_peers", || Value::Sequence(Vec::new()))?;
    let peers = peers
        .as_sequence_mut()
        .ok_or("`federation.trusted_peers` in NANDI.md isn't a list")?;

    let handle = peer.github_handle.trim();
    let matches = |entry: &Value| {
        entry
            .get("github_handle")
            .and_then(Value::as_str)
            .is_some_and(|h| h.trim().eq_ignore_ascii_case(handle))
    };
    let present = peers.iter().any(matches);
    match (trusted, present) {
        (true, true) | (false, false) => return Ok(None),
        (true, false) => {
            let mut entry = Mapping::new();
            entry.insert(Value::from("github_handle"), Value::from(handle));
            entry.insert(
                Value::from("display_name"),
                Value::from(peer.display_name.trim()),
            );
            peers.push(Value::Mapping(entry));
        }
        (false, true) => peers.retain(|entry| !matches(entry)),
    }
    let yaml = serde_yaml::to_string(&root).map_err(|e| format!("Encoding NANDI.md: {e}"))?;
    Ok(Some(format!("---\n{yaml}---{tail}")))
}

/// `parent[key]`, created with `empty()` when missing or null
fn child<'a>(
    parent: &'a mut Value,
    key: &str,
    empty: impl FnOnce() -> Value,
) -> Result<&'a mut Value, String> {
    let map = parent
        .as_mapping_mut()
        .ok_or_else(|| format!("NANDI.md: the parent of `{key}` isn't a mapping"))?;
    let value = map.entry(Value::from(key)).or_insert_with(|| Value::Null);
    if value.is_null() {
        *value = empty();
    }
    Ok(value)
}

/// `(frontmatter YAML, everything after the closing ---)`: the tail starts
/// with the newline that ends the closing line (or is empty)
fn split_frontmatter(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix("---")?;
    let start = 3 + if rest.starts_with("\r\n") {
        2
    } else if rest.starts_with('\n') {
        1
    } else {
        return None;
    };
    let mut line_start = start;
    loop {
        let line = &text[line_start..];
        if let Some(after) = line.strip_prefix("---") {
            if after.is_empty() || after.starts_with('\n') || after.starts_with("\r\n") {
                return Some((&text[start..line_start], after));
            }
        }
        line_start += line.find('\n')? + 1;
    }
}

/// Applies `edit_trusted_peers` to the file at `path`, atomically.
/// `Ok(false)`: nothing to do (no file yet, or already as asked).
pub fn set_trusted_peer_in_file(
    path: &Path,
    peer: &OwnerRef,
    trusted: bool,
) -> Result<bool, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(format!("Failed to read NANDI.md: {e}")),
    };
    match edit_trusted_peers(&text, peer, trusted)? {
        Some(updated) => {
            fs_atomic::write_atomic(path, updated.as_bytes())
                .map_err(|e| format!("Failed to write NANDI.md: {e}"))?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Who owns `instance_id`, from its gateway card in the directory
async fn peer_owner(state: &AppState, instance_id: &str) -> Option<OwnerRef> {
    let snapshot = network::get_network_snapshot(state).await.ok()?;
    snapshot
        .members
        .into_iter()
        .find(|member| member.instance_id == instance_id)
        .map(|member| OwnerRef {
            github_handle: member.handle.unwrap_or_default(),
            display_name: member.display_name,
        })
        .filter(|owner| !owner.github_handle.is_empty())
}

// ---------------------------------------------------------------------------
// Service functions
// ---------------------------------------------------------------------------

fn require_instance(instance_id: &str) -> Result<&str, String> {
    let instance_id = instance_id.trim();
    if instance_id.is_empty() {
        Err("Choose a member".to_string())
    } else {
        Ok(instance_id)
    }
}

/// Accepts an agent ID or its card ID (`<instance>/<agent>`)
fn agent_id_of(instance_id: &str, agent_id: &str) -> Result<String, String> {
    let agent_id = agent_id.trim();
    let prefix = format!("{instance_id}/");
    let agent_id = agent_id.strip_prefix(&prefix).unwrap_or(agent_id);
    if agent_id.is_empty() {
        Err("Choose an agent".to_string())
    } else {
        Ok(agent_id.to_string())
    }
}

fn updated(state: &AppState, relationship: TrustRelationship) -> TrustRelationship {
    emit(state.bus(), &relationship);
    relationship
}

pub async fn set_trust_relationship(
    state: &AppState,
    instance_id: &str,
    update: TrustUpdate,
) -> Result<TrustRelationship, String> {
    let instance_id = require_instance(instance_id)?;
    let before = get_relationship(state.db(), instance_id)?;
    let relationship = update_relationship(state.db(), instance_id, &update)?;
    if relationship.added_as_trusted_peer != before.added_as_trusted_peer {
        match peer_owner(state, instance_id).await {
            Some(owner) => {
                let path = manifest::manifest_path(&state.store().layout().root);
                if let Err(e) =
                    set_trusted_peer_in_file(&path, &owner, relationship.added_as_trusted_peer)
                {
                    log::warn!("Trusted peer saved, but NANDI.md wasn't updated: {e}");
                }
            }
            None => {
                log::info!("Trusted peer {instance_id} has no handle yet; NANDI.md left unchanged")
            }
        }
    }
    Ok(updated(state, relationship))
}

pub async fn block_agent_from_member(
    state: &AppState,
    instance_id: &str,
    agent_id: &str,
) -> Result<TrustRelationship, String> {
    let instance_id = require_instance(instance_id)?;
    let agent_id = agent_id_of(instance_id, agent_id)?;
    let relationship = set_blocked(state.db(), instance_id, &agent_id, true)?;
    Ok(updated(state, relationship))
}

pub async fn unblock_agent_from_member(
    state: &AppState,
    instance_id: &str,
    agent_id: &str,
) -> Result<TrustRelationship, String> {
    let instance_id = require_instance(instance_id)?;
    let agent_id = agent_id_of(instance_id, agent_id)?;
    let relationship = set_blocked(state.db(), instance_id, &agent_id, false)?;
    Ok(updated(state, relationship))
}

pub async fn pause_delegations_from_member(
    state: &AppState,
    instance_id: &str,
    paused: bool,
) -> Result<TrustRelationship, String> {
    let instance_id = require_instance(instance_id)?;
    let relationship = update_relationship(
        state.db(),
        instance_id,
        &TrustUpdate {
            added_as_trusted_peer: None,
            delegations_paused: Some(paused),
        },
    )?;
    Ok(updated(state, relationship))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(instance: &str) -> network::NetworkMember {
        network::member_from_card(
            crate::services::gateway::GatewayCard {
                instance_id: instance.to_string(),
                endpoint: String::new(),
                display_name: instance.to_string(),
                description: String::new(),
                capabilities: Vec::new(),
                accepts_delegation: true,
                max_concurrent: 1,
                current_load: 0,
                owner: OwnerRef::default(),
                federation_enabled: false,
                gateway_version: 1,
                agent_count: 0,
                body: None,
                direct_delegation: None,
                responsibilities: Vec::new(),
                availability: Default::default(),
            },
            0.0,
        )
    }

    fn set_token(db: &Db, token: Option<&str>) {
        let conn = db.conn().unwrap();
        match token {
            Some(token) => conn
                .execute(
                    "INSERT INTO relay_state (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![
                        crate::services::relay::RELAY_TOKEN_KEY,
                        serde_json::json!(token).to_string()
                    ],
                )
                .unwrap(),
            None => conn
                .execute(
                    "DELETE FROM relay_state WHERE key = ?1",
                    [crate::services::relay::RELAY_TOKEN_KEY],
                )
                .unwrap(),
        };
    }

    #[test]
    fn everyone_on_a_secured_relay_is_trusted_unless_turned_off() {
        let db = Db::in_memory();
        network::remember_members(&db, &[member("lena")]);

        // An open relay (no token): trust stays manual
        assert!(!on_a_secured_relay(&db));
        assert!(!is_trusted(&db, "lena"));

        // The company's relay: its members are trusted, strangers aren't
        set_token(&db, Some("tok"));
        assert!(is_trusted(&db, "lena"));
        assert!(!is_trusted(&db, "stranger"));
        assert_eq!(
            relay_trust(&db),
            RelayTrust {
                enabled: true,
                secured: true,
                public: false
            }
        );

        // Turned off: only those trusted by hand
        set_trusts_relay_members(&db, false).unwrap();
        assert!(!is_trusted(&db, "lena"));
        update_relationship(
            &db,
            "lena",
            &TrustUpdate {
                added_as_trusted_peer: Some(true),
                delegations_paused: None,
            },
        )
        .unwrap();
        assert!(is_trusted(&db, "lena"));
        set_token(&db, None);
    }

    fn peer(handle: &str, name: &str) -> OwnerRef {
        OwnerRef {
            github_handle: handle.to_string(),
            display_name: name.to_string(),
        }
    }

    const NANDI_MD: &str = "---\nname: \"Me's Nandi\"\ndisplay_name: Me\nrelay:\n  enabled: true\n  discoverable: false\npolicy:\n  max_concurrent: 3\n  expose: []\nfederation:\n  enabled: false\n  trusted_peers: []\nrouting:\n  preferences: {}\n---\n\nI'm Me.\n\n## What I can help with\n\n---not a fence\nPapers.\n";

    #[test]
    fn trusted_peers_edits_keep_everything_else() {
        let added = edit_trusted_peers(NANDI_MD, &peer("alice", "Alice"), true)
            .unwrap()
            .unwrap();
        let (yaml, tail) = split_frontmatter(&added).unwrap();
        // The body is untouched, byte for byte
        let (_, original_tail) = split_frontmatter(NANDI_MD).unwrap();
        assert_eq!(tail, original_tail);
        assert!(tail.ends_with("---not a fence\nPapers.\n"));
        let value: Value = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(value["name"].as_str(), Some("Me's Nandi"));
        assert_eq!(value["relay"]["enabled"].as_bool(), Some(true));
        assert_eq!(value["policy"]["max_concurrent"].as_u64(), Some(3));
        assert!(value["routing"]["preferences"].is_mapping());
        let peers = value["federation"]["trusted_peers"].as_sequence().unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0]["github_handle"].as_str(), Some("alice"));
        assert_eq!(peers[0]["display_name"].as_str(), Some("Alice"));
        // Key order survives
        let keys: Vec<&str> = value
            .as_mapping()
            .unwrap()
            .keys()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "name",
                "display_name",
                "relay",
                "policy",
                "federation",
                "routing"
            ]
        );

        // Adding again changes nothing; removing (any case) empties the list
        assert_eq!(
            edit_trusted_peers(&added, &peer("alice", "Alice"), true).unwrap(),
            None
        );
        let removed = edit_trusted_peers(&added, &peer("ALICE", ""), false)
            .unwrap()
            .unwrap();
        let (yaml, _) = split_frontmatter(&removed).unwrap();
        let value: Value = serde_yaml::from_str(yaml).unwrap();
        assert!(value["federation"]["trusted_peers"]
            .as_sequence()
            .unwrap()
            .is_empty());
        assert_eq!(value["federation"]["enabled"].as_bool(), Some(false));
        assert_eq!(
            edit_trusted_peers(&removed, &peer("alice", ""), false).unwrap(),
            None
        );
    }

    #[test]
    fn a_missing_federation_section_is_created() {
        let text = "---\nname: x\n---\nBody\n";
        let added = edit_trusted_peers(text, &peer("bob", "Bob"), true)
            .unwrap()
            .unwrap();
        assert!(added.ends_with("---\nBody\n"));
        let (yaml, _) = split_frontmatter(&added).unwrap();
        let value: Value = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(value["name"].as_str(), Some("x"));
        assert_eq!(
            value["federation"]["trusted_peers"][0]["github_handle"].as_str(),
            Some("bob")
        );
        assert!(edit_trusted_peers("no frontmatter", &peer("bob", ""), true).is_err());
        assert!(edit_trusted_peers("---\nname: x\n", &peer("bob", ""), true).is_err());
        // CRLF files and an empty frontmatter work too
        let crlf = "---\r\nname: x\r\n---\r\nBody\r\n";
        let edited = edit_trusted_peers(crlf, &peer("bob", ""), true)
            .unwrap()
            .unwrap();
        assert!(edited.ends_with("---\r\nBody\r\n"));
        assert!(edit_trusted_peers("---\n---\n", &peer("bob", ""), true)
            .unwrap()
            .is_some());
    }

    #[test]
    fn the_file_is_written_atomically_and_left_alone_when_missing() {
        let dir = std::env::temp_dir().join(format!("nandi-trust-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(manifest::MANIFEST_FILE);
        assert!(!set_trusted_peer_in_file(&path, &peer("a", "A"), true).unwrap());
        assert!(!path.exists());
        std::fs::write(&path, NANDI_MD).unwrap();
        assert!(set_trusted_peer_in_file(&path, &peer("a", "A"), true).unwrap());
        assert!(!set_trusted_peer_in_file(&path, &peer("a", "A"), true).unwrap());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("github_handle: a"));
        assert!(text.ends_with("---not a fence\nPapers.\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn relationships_upsert_and_checks_read_them() {
        let db = Db::in_memory();
        let empty = get_relationship(&db, "i-a").unwrap();
        assert_eq!(empty.instance_id, "i-a");
        assert!(!empty.added_as_trusted_peer && !empty.delegations_paused);
        assert!(empty.last_updated.is_none());
        assert!(!is_paused(&db, "i-a"));

        let paused = update_relationship(
            &db,
            "i-a",
            &TrustUpdate {
                added_as_trusted_peer: Some(true),
                delegations_paused: Some(true),
            },
        )
        .unwrap();
        assert!(paused.added_as_trusted_peer && paused.delegations_paused);
        assert!(is_paused(&db, "i-a"));
        // `None` keeps the value
        let kept = update_relationship(
            &db,
            "i-a",
            &TrustUpdate {
                added_as_trusted_peer: None,
                delegations_paused: Some(false),
            },
        )
        .unwrap();
        assert!(kept.added_as_trusted_peer && !kept.delegations_paused);
        assert!(!is_paused(&db, "i-a"));

        let blocked = set_blocked(&db, "i-a", "papers", true).unwrap();
        let twice = set_blocked(&db, "i-a", "papers", true).unwrap();
        assert_eq!(twice.blocked_agents, ["papers"]);
        let unblocked = set_blocked(&db, "i-b", "papers", true)
            .and_then(|_| set_blocked(&db, "i-b", "papers", false))
            .unwrap();
        assert!(unblocked.blocked_agents.is_empty());
        assert!(
            unblocked.last_updated.is_some(),
            "an unblock is a change too"
        );
        assert!(blocked.last_updated <= twice.last_updated);
        assert!(is_blocked(&db, "i-a", "papers"));
        assert!(is_blocked(&db, "i-a", "i-a/papers"));
        assert!(!is_blocked(&db, "i-b", "papers"));
        let filter = TrustFilter::load(&db).unwrap();
        assert!(filter.is_blocked("i-a", "papers") && !filter.is_paused("i-a"));
        let unblocked = set_blocked(&db, "i-a", "papers", false).unwrap();
        assert!(unblocked.blocked_agents.is_empty());
        assert!(!is_blocked(&db, "i-a", "papers"));

        // The whole instance
        set_blocked(&db, "i-a", ALL_AGENTS, true).unwrap();
        assert!(is_instance_blocked(&db, "i-a"));
        assert!(is_blocked(&db, "i-a", "papers") && is_blocked(&db, "i-a", "anything"));
        assert!(!is_instance_blocked(&db, "i-b"));
        let filter = TrustFilter::load(&db).unwrap();
        assert!(filter.is_blocked("i-a", "papers") && !filter.is_blocked("i-b", "papers"));
    }

    #[tokio::test]
    async fn service_functions_emit_and_validate() {
        use crate::services::events::tests::RecordingSink;
        let sink = std::sync::Arc::new(RecordingSink::default());
        let state = AppState::for_tests(vec![sink.clone()]);
        assert!(state.trust().is_some());
        let rel = pause_delegations_from_member(&state, "i-x", true)
            .await
            .unwrap();
        assert!(rel.delegations_paused);
        let rel = block_agent_from_member(&state, "i-x", "i-x/papers")
            .await
            .unwrap();
        assert_eq!(rel.blocked_agents, ["papers"]);
        let rel = unblock_agent_from_member(&state, "i-x", "papers")
            .await
            .unwrap();
        assert!(rel.blocked_agents.is_empty());
        // Trusting a member the directory doesn't know: saved, NANDI.md untouched
        let rel = set_trust_relationship(
            &state,
            "i-x",
            TrustUpdate {
                added_as_trusted_peer: Some(true),
                delegations_paused: None,
            },
        )
        .await
        .unwrap();
        assert!(rel.added_as_trusted_peer && rel.delegations_paused);
        assert!(block_agent_from_member(&state, " ", "a").await.is_err());
        assert!(block_agent_from_member(&state, "i-x", " ").await.is_err());
        let updates = sink
            .names()
            .into_iter()
            .filter(|n| *n == TRUST_RELATIONSHIP_UPDATED)
            .count();
        assert_eq!(updates, 4);
        state.shutdown();
    }
}
