//! A person's profile, by their instance: what their gateway card says,
//! the agents they put online at the relay, our delegations with them
//! (both directions), our trust toward them, and the key we pinned.
//!
//! Only gateway card fields and agents they made callable are shown: never
//! anything private to the member.

use std::collections::HashSet;

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{self, now_ms, Db};
use crate::model::Timestamp;
use crate::services::a2a::task::{TaskRecord, TaskState};
use crate::services::domain::AgentCard;
use crate::services::gateway::GatewayCard;
use crate::services::network::member_trust::{self as trust, TrustRelationship};
use crate::services::network::peer_keys::{self, PeerKeyStatus};
use crate::services::network::{self, NetworkMember, NetworkSnapshot};
use crate::services::{relay, AppState};

/// `member://status_changed`
pub const MEMBER_STATUS_CHANGED: &str = "member://status_changed";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DelegationStats {
    /// Tasks we sent them
    pub sent: u32,
    /// Tasks they sent us
    pub received: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub avg_duration_ms: Option<u32>,
    pub last_at: Option<Timestamp>,
}

/// One of a member's agents, online at the relay
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MemberAgent {
    pub card: AgentCard,
    /// We blocked it (`blocked_members`)
    pub blocked: bool,
}

/// Everything the member profile shows. `connected` false: they haven't
/// connected their Nandi yet (no gateway card).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MemberProfile {
    pub instance_id: String,
    pub handle: Option<String>,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub online: bool,
    pub last_seen: Option<Timestamp>,
    pub connected: bool,
    pub gateway: Option<GatewayCard>,
    pub capabilities: Vec<String>,
    pub agents: Vec<MemberAgent>,
    pub delegation_stats: DelegationStats,
    /// Newest first, both directions
    pub recent_delegations: Vec<TaskRecord>,
    pub trust: TrustRelationship,
    /// The key we pinned for them; `None` before the directory showed one
    pub key: Option<PeerKeyStatus>,
}

/// How many tasks a profile lists
pub const RECENT_DELEGATIONS: u32 = 20;

// ---------------------------------------------------------------------------
// Tasks (read-only: the task lifecycle writes them)
// ---------------------------------------------------------------------------

const TASK_COLUMNS: &str = "id, requester_instance, requester_agent, provider_instance,
    provider_agent, capability, input, state, eta_seconds, deadline, priority, created_at,
    updated_at, acknowledged_at, completed_at, error, result, parent_task_id, source_kind,
    attempt_number, previous_attempt_id";

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRecord> {
    let json = |text: Option<String>| text.and_then(|t| serde_json::from_str(&t).ok());
    let ms = |value: Option<i64>| value.map(|v| v as Timestamp);
    Ok(TaskRecord {
        id: row.get(0)?,
        requester_instance: row.get(1)?,
        requester_agent: row.get(2)?,
        provider_instance: row.get(3)?,
        provider_agent: row.get(4)?,
        capability: row.get(5)?,
        input: json(row.get(6)?).unwrap_or(serde_json::Value::Null),
        state: db::text_enum(row, 7)?,
        eta_seconds: row
            .get::<_, Option<i64>>(8)?
            .and_then(|s| u32::try_from(s).ok()),
        deadline: ms(row.get(9)?),
        priority: row.get(10)?,
        created_at: row.get::<_, i64>(11)? as Timestamp,
        updated_at: row.get::<_, i64>(12)? as Timestamp,
        acknowledged_at: ms(row.get(13)?),
        completed_at: ms(row.get(14)?),
        error: row.get(15)?,
        result: json(row.get(16)?),
        parent_task_id: row.get(17)?,
        source_kind: db::text_enum(row, 18)?,
        attempt_number: u32::try_from(row.get::<_, i64>(19)?).unwrap_or(1),
        previous_attempt_id: row.get(20)?,
    })
}

/// States that count as a failed delegation (cancelled ones don't)
const FAILED_STATES: [TaskState; 4] = [
    TaskState::Failed,
    TaskState::Rejected,
    TaskState::TimedOut,
    TaskState::Abandoned,
];

/// Tasks between us (`local`) and `theirs`, in both directions
pub fn delegation_stats(db: &Db, local: &str, theirs: &str) -> Result<DelegationStats, String> {
    let conn = db.conn()?;
    let failed = FAILED_STATES
        .iter()
        .map(|s| format!("'{}'", s.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    let completed = TaskState::Completed.as_str();
    let sql = format!(
        "SELECT
           COALESCE(SUM(requester_instance = ?1 AND provider_instance = ?2), 0),
           COALESCE(SUM(requester_instance = ?2 AND provider_instance = ?1), 0),
           COALESCE(SUM(state = '{completed}'), 0),
           COALESCE(SUM(state IN ({failed})), 0),
           AVG(CASE WHEN state = '{completed}' AND completed_at IS NOT NULL
                    THEN completed_at - created_at END),
           MAX(created_at)
         FROM tasks
         WHERE (requester_instance = ?1 AND provider_instance = ?2)
            OR (requester_instance = ?2 AND provider_instance = ?1)"
    );
    conn.query_row(&sql, params![local, theirs], |row| {
        let count = |v: i64| u32::try_from(v).unwrap_or(u32::MAX);
        Ok(DelegationStats {
            sent: count(row.get(0)?),
            received: count(row.get(1)?),
            succeeded: count(row.get(2)?),
            failed: count(row.get(3)?),
            avg_duration_ms: row
                .get::<_, Option<f64>>(4)?
                .map(|ms| ms.max(0.0).round().min(u32::MAX as f64) as u32),
            last_at: row.get::<_, Option<i64>>(5)?.map(|ms| ms as Timestamp),
        })
    })
    .map_err(|e| format!("Database error: {e}"))
}

/// The newest `limit` tasks between us and `theirs`, both directions
pub fn recent_delegations(
    db: &Db,
    local: &str,
    theirs: &str,
    limit: u32,
) -> Result<Vec<TaskRecord>, String> {
    let conn = db.conn()?;
    let sql = format!(
        "SELECT {TASK_COLUMNS} FROM tasks
          WHERE (requester_instance = ?1 AND provider_instance = ?2)
             OR (requester_instance = ?2 AND provider_instance = ?1)
          ORDER BY created_at DESC, id LIMIT ?3"
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("Database error: {e}"))?;
    let rows = stmt
        .query_map(params![local, theirs, limit], task_from_row)
        .map_err(|e| format!("Database error: {e}"))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| format!("Database error: {e}"))
}

// ---------------------------------------------------------------------------
// Profiles
// ---------------------------------------------------------------------------

/// What one instance's profile is built from
pub struct ProfileSources<'a> {
    pub instance_id: &'a str,
    /// Its entry in the directory, when it has one
    pub member: Option<&'a NetworkMember>,
    /// Its agents online at the relay
    pub cards: Vec<AgentCard>,
    pub trust: TrustRelationship,
    pub delegation_stats: DelegationStats,
    pub recent_delegations: Vec<TaskRecord>,
}

/// Everything known about one instance, from what it shares only
pub fn build_profile(sources: ProfileSources<'_>) -> MemberProfile {
    let ProfileSources {
        instance_id,
        member,
        cards,
        trust,
        delegation_stats,
        recent_delegations,
    } = sources;
    let blocked: HashSet<&str> = trust.blocked_agents.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let mut agents: Vec<MemberAgent> = cards
        .into_iter()
        .filter(|card| seen.insert(card.id.clone()))
        .map(|card| MemberAgent {
            blocked: blocked.contains(card.agent_id()),
            card,
        })
        .collect();
    agents.sort_by(|a, b| {
        a.card
            .name
            .to_lowercase()
            .cmp(&b.card.name.to_lowercase())
            .then_with(|| a.card.id.cmp(&b.card.id))
    });

    let card_owner = agents
        .iter()
        .map(|a| &a.card.owner)
        .find(|owner| !owner.github_handle.is_empty());
    let handle = member
        .and_then(|m| m.handle.clone())
        .or_else(|| card_owner.map(|o| o.github_handle.clone()))
        .filter(|h| !h.trim().is_empty());
    let display_name = [
        member.map(|m| m.display_name.as_str()),
        card_owner.map(|o| o.display_name.as_str()),
        handle.as_deref(),
        Some(instance_id),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .find(|name| !name.is_empty())
    .unwrap_or_default()
    .to_string();
    let capabilities = match member {
        Some(member) => member.gateway.capabilities.clone(),
        None => {
            let mut names: Vec<String> = agents
                .iter()
                .flat_map(|a| a.card.capabilities.iter().map(|c| c.name.clone()))
                .collect();
            names.sort();
            names.dedup();
            names
        }
    };
    MemberProfile {
        instance_id: instance_id.to_string(),
        handle,
        display_name,
        avatar_url: member.and_then(|m| m.avatar_url.clone()),
        online: member.is_some_and(|m| m.online),
        last_seen: member.and_then(|m| m.last_seen),
        connected: member.is_some(),
        gateway: member.map(|m| m.gateway.clone()),
        capabilities,
        agents,
        delegation_stats,
        recent_delegations,
        trust,
        key: None,
    }
}

// ---------------------------------------------------------------------------
// Service functions
// ---------------------------------------------------------------------------

/// The directory, or an empty one when it can't be read
async fn directory(state: &AppState) -> NetworkSnapshot {
    network::get_network_snapshot(state)
        .await
        .unwrap_or_else(|e| {
            log::debug!("Profile: no network directory: {e}");
            NetworkSnapshot {
                relay_url: None,
                relay_connected: false,
                members: Vec::new(),
                fetched_at: now_ms() as Timestamp,
            }
        })
}

pub async fn get_member_by_instance(
    state: &AppState,
    instance_id: &str,
) -> Result<MemberProfile, String> {
    let instance_id = instance_id.trim();
    if instance_id.is_empty() {
        return Err("Choose a member".to_string());
    }
    let snapshot = directory(state).await;
    profile(state, instance_id, &snapshot).await
}

async fn profile(
    state: &AppState,
    instance_id: &str,
    snapshot: &NetworkSnapshot,
) -> Result<MemberProfile, String> {
    let member = snapshot
        .members
        .iter()
        .find(|m| m.instance_id == instance_id);
    let cards: Vec<AgentCard> = relay::list_online_agents(state, None)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|card| card.instance_id == instance_id)
        .collect();
    let local = network::local_instance_id(state.db())?;
    let mut profile = build_profile(ProfileSources {
        instance_id,
        member,
        cards,
        trust: trust::get_relationship(state.db(), instance_id)?,
        delegation_stats: delegation_stats(state.db(), &local, instance_id)?,
        recent_delegations: recent_delegations(
            state.db(),
            &local,
            instance_id,
            RECENT_DELEGATIONS,
        )?,
    });
    profile.key = peer_keys::status(state.db(), instance_id)?;
    Ok(profile)
}

/// For tests elsewhere: a stored task row between two instances
#[cfg(test)]
pub(crate) fn insert_test_task(
    db: &Db,
    id: &str,
    requester: &str,
    provider: &str,
    state: TaskState,
    created_at: i64,
    completed_at: Option<i64>,
) {
    db.conn()
        .unwrap()
        .execute(
            "INSERT INTO tasks (id, requester_instance, requester_agent, provider_instance,
               provider_agent, capability, state, created_at, updated_at, completed_at)
             VALUES (?1, ?2, '_user', ?3, 'papers', 'find-papers', ?4, ?5, ?5, ?6)",
            params![
                id,
                requester,
                provider,
                state.as_str(),
                created_at,
                completed_at
            ],
        )
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::agent_card::{card_from_agent, OwnerRef};
    use crate::services::domain::{Agent, AgentId};

    fn card(instance: &str, agent: &str, handle: &str) -> AgentCard {
        let skill = format!("---\nname: {agent}\nserves: [{{name: find-papers}}]\n---\n");
        let def = Agent::from_skill_md(AgentId::parse(agent).unwrap(), &skill).unwrap();
        card_from_agent(&def, instance, "").with_owner(OwnerRef {
            github_handle: handle.to_string(),
            display_name: format!("{handle} name"),
        })
    }

    fn member(instance: &str, handle: &str, online: bool) -> NetworkMember {
        NetworkMember {
            instance_id: instance.to_string(),
            display_name: format!("{handle} gateway"),
            avatar_url: None,
            online,
            last_seen: Some(1.0),
            gateway: GatewayCard {
                instance_id: instance.to_string(),
                endpoint: String::new(),
                display_name: format!("{handle} gateway"),
                description: String::new(),
                capabilities: vec!["summarize".to_string()],
                accepts_delegation: true,
                max_concurrent: 1,
                current_load: 0,
                owner: OwnerRef {
                    github_handle: handle.to_string(),
                    display_name: handle.to_string(),
                },
                federation_enabled: false,
                gateway_version: 1,
                agent_count: 1,
                body: Some("I help.".to_string()),
                direct_delegation: None,
                responsibilities: Vec::new(),
                availability: Default::default(),
            },
            handle: Some(handle.to_string()),
        }
    }

    fn sources<'a>(
        member: Option<&'a NetworkMember>,
        cards: Vec<AgentCard>,
        trust: TrustRelationship,
    ) -> ProfileSources<'a> {
        ProfileSources {
            instance_id: "i-a",
            member,
            cards,
            trust,
            delegation_stats: DelegationStats::default(),
            recent_delegations: Vec::new(),
        }
    }

    #[test]
    fn delegation_stats_count_both_directions() {
        let db = Db::in_memory();
        insert_test_task(
            &db,
            "1",
            "me",
            "i-a",
            TaskState::Completed,
            1000,
            Some(3000),
        );
        insert_test_task(&db, "2", "me", "i-a", TaskState::Failed, 2000, None);
        insert_test_task(
            &db,
            "3",
            "i-a",
            "me",
            TaskState::Completed,
            4000,
            Some(5000),
        );
        insert_test_task(&db, "4", "i-a", "me", TaskState::Cancelled, 6000, None);
        insert_test_task(
            &db,
            "5",
            "me",
            "i-b",
            TaskState::Completed,
            7000,
            Some(8000),
        );
        let stats = delegation_stats(&db, "me", "i-a").unwrap();
        assert_eq!((stats.sent, stats.received), (2, 2));
        assert_eq!((stats.succeeded, stats.failed), (2, 1));
        assert_eq!(stats.avg_duration_ms, Some(1500));
        assert_eq!(stats.last_at, Some(6000.0));
        let none = delegation_stats(&db, "me", "i-z").unwrap();
        assert_eq!(none, DelegationStats::default());

        let recent = recent_delegations(&db, "me", "i-a", 3).unwrap();
        let ids: Vec<&str> = recent.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["4", "3", "2"]);
        assert_eq!(recent[1].state, TaskState::Completed);
        assert_eq!(recent[1].completed_at, Some(5000.0));
    }

    #[test]
    fn profiles_list_agents_once_by_name_and_mark_blocked_ones() {
        let trust = TrustRelationship {
            instance_id: "i-a".into(),
            blocked_agents: vec!["notes".into()],
            ..TrustRelationship::default()
        };
        let alice = member("i-a", "alice", true);
        let profile = build_profile(sources(
            Some(&alice),
            vec![
                card("i-a", "papers", "alice"),
                card("i-a", "notes", "alice"),
                card("i-a", "papers", "alice"),
            ],
            trust,
        ));
        assert!(profile.connected && profile.online);
        assert_eq!(profile.handle.as_deref(), Some("alice"));
        assert_eq!(profile.capabilities, ["summarize"]);
        let agents: Vec<(&str, bool)> = profile
            .agents
            .iter()
            .map(|a| (a.card.id.as_str(), a.blocked))
            .collect();
        assert_eq!(agents, [("i-a/notes", true), ("i-a/papers", false)]);
    }

    #[test]
    fn a_member_without_a_gateway_is_not_connected() {
        let profile = build_profile(sources(None, Vec::new(), TrustRelationship::default()));
        assert!(!profile.connected && !profile.online);
        assert!(profile.gateway.is_none() && profile.agents.is_empty());
        assert_eq!(profile.display_name, "i-a");
        assert_eq!(profile.handle, None);

        // Known only by its agents: capabilities and names come from them
        let profile = build_profile(sources(
            None,
            vec![card("i-a", "papers", "carol")],
            TrustRelationship::default(),
        ));
        assert!(!profile.connected);
        assert_eq!(profile.capabilities, ["find-papers"]);
        assert_eq!(profile.display_name, "carol name");
        assert_eq!(profile.handle.as_deref(), Some("carol"));
    }

    #[tokio::test]
    async fn an_unknown_instance_gets_the_empty_state() {
        let state = AppState::for_tests(vec![]);
        let profile = get_member_by_instance(&state, "i-nobody").await.unwrap();
        assert!(!profile.connected);
        assert_eq!(profile.instance_id, "i-nobody");
        assert_eq!(profile.display_name, "i-nobody");
        assert_eq!(profile.trust.instance_id, "i-nobody");
        assert!(get_member_by_instance(&state, " ").await.is_err());
        state.shutdown();
    }
}
