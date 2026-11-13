//! Who may call which local agent: per-edge `allow` / `deny` / `ask`.
//! The trust view edits the policies (TrustGraph); delegation enforces
//! them on inbound tasks. Machine-local, like pilot grants: the `trust_policies`
//! table, not the gold copy.
//!
//! The lookup and write below are what both sides build on.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{now_ms, Db};
use crate::model::Timestamp;

/// Matches every requester
#[cfg(test)]
pub const ANY_REQUESTER: &str = "*";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum TrustPolicy {
    Allow,
    Deny,
    /// The user approves each call (the default for unknown callers)
    #[default]
    Ask,
}

impl TrustPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Ask => "ask",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            "ask" => Some(Self::Ask),
            _ => None,
        }
    }
}

/// One edge: `requester` (a card ID, or `*`) calling `provider_agent` (a
/// local agent ID).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TrustRule {
    pub requester: String,
    pub provider_agent: String,
    pub policy: TrustPolicy,
    pub updated_at: Timestamp,
}

/// The policy for `requester` calling `provider_agent`: its own rule, else
/// the `*` rule for that agent, else `Ask`.
pub fn policy_for(db: &Db, requester: &str, provider_agent: &str) -> Result<TrustPolicy, String> {
    let conn = db.conn()?;
    let policy: Option<String> = conn
        .query_row(
            "SELECT policy FROM trust_policies
              WHERE provider_agent = ?2 AND requester IN (?1, '*')
              ORDER BY requester = '*' LIMIT 1",
            params![requester, provider_agent],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Database error: {e}"))?;
    Ok(policy
        .as_deref()
        .and_then(TrustPolicy::parse)
        .unwrap_or_default())
}

pub fn set_policy(
    db: &Db,
    requester: &str,
    provider_agent: &str,
    policy: TrustPolicy,
) -> Result<(), String> {
    let conn = db.conn()?;
    conn.execute(
        "INSERT INTO trust_policies (requester, provider_agent, policy, updated_at)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(requester, provider_agent)
         DO UPDATE SET policy = excluded.policy, updated_at = excluded.updated_at",
        params![requester, provider_agent, policy.as_str(), now_ms()],
    )
    .map_err(|e| format!("Database error: {e}"))?;
    Ok(())
}

pub fn list_policies(db: &Db) -> Result<Vec<TrustRule>, String> {
    let conn = db.conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT requester, provider_agent, policy, updated_at FROM trust_policies
              ORDER BY provider_agent, requester",
        )
        .map_err(|e| format!("Database error: {e}"))?;
    let rows = stmt
        .query_map([], |row| {
            let policy: String = row.get(2)?;
            let updated_at: i64 = row.get(3)?;
            Ok(TrustRule {
                requester: row.get(0)?,
                provider_agent: row.get(1)?,
                policy: TrustPolicy::parse(&policy).unwrap_or_default(),
                updated_at: updated_at as Timestamp,
            })
        })
        .map_err(|e| format!("Database error: {e}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| format!("Database error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specific_rules_beat_the_wildcard_and_ask_is_the_default() {
        let db = Db::in_memory();
        assert_eq!(policy_for(&db, "i/a", "papers").unwrap(), TrustPolicy::Ask);
        set_policy(&db, ANY_REQUESTER, "papers", TrustPolicy::Deny).unwrap();
        assert_eq!(policy_for(&db, "i/a", "papers").unwrap(), TrustPolicy::Deny);
        set_policy(&db, "i/a", "papers", TrustPolicy::Allow).unwrap();
        assert_eq!(
            policy_for(&db, "i/a", "papers").unwrap(),
            TrustPolicy::Allow
        );
        assert_eq!(policy_for(&db, "i/b", "papers").unwrap(), TrustPolicy::Deny);
        assert_eq!(list_policies(&db).unwrap().len(), 2);
    }
}
