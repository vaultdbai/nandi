//! Team rosters and member profiles (wave final, Track E): per member, the
//! gateway card, what they shared, how many teams we share, and delegation
//! stats. Rosters are cached for 60s (`roster_cache`); members are matched
//! to instances through the network directory (gateway cards) and the
//! instance IDs on their shared cards.
//!
//! Only gateway card fields and explicitly shared registry items are
//! shown: never anything private to the member.

use std::collections::{BTreeMap, HashSet};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{now_ms, Db};
use crate::model::Timestamp;
use crate::services::a2a::task::{TaskRecord, TaskSourceKind, TaskState};
use crate::services::domain::AgentCard;
use crate::services::events::{emit, NandiEvent};
use crate::services::gateway::{CatalogWorkflow, GatewayCard};
use crate::services::network::peer_keys::{self, PeerKeyStatus};
use crate::services::network::{self, NetworkMember, NetworkSnapshot};
use crate::services::{relay, AppState};

use super::trust::{self, TrustRelationship};
use super::{Team, TeamManager};

/// `team://roster_changed`
pub const TEAM_ROSTER_CHANGED: &str = "team://roster_changed";
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct RosterMember {
    pub handle: String,
    pub display_name: String,
    pub instance_id: Option<String>,
    pub online: bool,
    pub gateway: Option<GatewayCard>,
    pub shared_agents: Vec<AgentCard>,
    pub shared_workflows: Vec<CatalogWorkflow>,
    /// Teams we're both in
    pub team_count: u32,
    pub delegation_stats: DelegationStats,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TeamRoster {
    pub team_id: String,
    pub members: Vec<RosterMember>,
    pub fetched_at: Timestamp,
}

/// Where we can reach one of a member's agents from
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum MemberAgentSource {
    /// Shared to a team we're in
    Team,
    /// On their gateway card, or shared with us directly
    Direct,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MemberAgent {
    pub card: AgentCard,
    pub source: MemberAgentSource,
    pub team_id: Option<String>,
    /// We blocked it (`blocked_members`)
    pub blocked: bool,
}

/// Everything the member profile shows. `connected` false: they haven't
/// connected their Nandi yet (no gateway card).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MemberProfile {
    pub instance_id: Option<String>,
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

/// Rosters are served from `roster_cache` this long
pub const ROSTER_TTL_MS: i64 = 60_000;
/// How many tasks a profile lists
pub const RECENT_DELEGATIONS: u32 = 20;

/// Payload of `team://roster_changed`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TeamRosterChanged {
    pub team_id: String,
}

impl NandiEvent for TeamRosterChanged {
    const NAME: &'static str = TEAM_ROSTER_CHANGED;
}

/// The roster service: rosters live in `roster_cache`, so this only keeps
/// the cache honest (see `init`).
pub struct RosterService {
    _private: (),
}

/// Drops cached rosters when a team changes (emitting
/// `team://roster_changed`) or a member comes or goes. Never blocks.
pub fn init(state: &AppState) -> Option<std::sync::Arc<RosterService>> {
    let mut events = state.broadcast().subscribe();
    let db = state.db().clone();
    let bus = state.bus().clone();
    state.runtime().spawn(async move {
        use tokio::sync::broadcast::error::RecvError;
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            };
            match event.event {
                "team://changed" => {
                    let Some(team_id) = event.payload.get("teamId").and_then(|v| v.as_str()) else {
                        continue;
                    };
                    if let Err(e) = clear_cache(&db, Some(team_id)) {
                        log::debug!("Roster cache: {e}");
                    }
                    emit(
                        &bus,
                        &TeamRosterChanged {
                            team_id: team_id.to_string(),
                        },
                    );
                }
                network::NETWORK_MEMBER_ONLINE
                | network::NETWORK_MEMBER_OFFLINE
                | network::NETWORK_MEMBERSHIP_CHANGED => {
                    if let Err(e) = clear_cache(&db, None) {
                        log::debug!("Roster cache: {e}");
                    }
                }
                _ => {}
            }
        }
    });
    Some(std::sync::Arc::new(RosterService { _private: () }))
}

// ---------------------------------------------------------------------------
// Tasks (read-only: Track B writes them)
// ---------------------------------------------------------------------------

const TASK_COLUMNS: &str = "id, requester_instance, requester_agent, provider_instance,
    provider_agent, capability, input, state, eta_seconds, deadline, priority, created_at,
    updated_at, acknowledged_at, completed_at, error, result, parent_task_id, source_kind,
    attempt_number, previous_attempt_id";

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRecord> {
    let json = |text: Option<String>| text.and_then(|t| serde_json::from_str(&t).ok());
    let ms = |value: Option<i64>| value.map(|v| v as Timestamp);
    let state: String = row.get(7)?;
    let source: String = row.get(18)?;
    Ok(TaskRecord {
        id: row.get(0)?,
        requester_instance: row.get(1)?,
        requester_agent: row.get(2)?,
        provider_instance: row.get(3)?,
        provider_agent: row.get(4)?,
        capability: row.get(5)?,
        input: json(row.get(6)?).unwrap_or(serde_json::Value::Null),
        state: TaskState::parse(&state).unwrap_or(TaskState::Failed),
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
        source_kind: TaskSourceKind::parse(&source),
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
// Cache
// ---------------------------------------------------------------------------

/// The cached roster, if fetched within the last 60s
fn cached_roster(db: &Db, team_id: &str, now: i64) -> Result<Option<Vec<RosterMember>>, String> {
    let conn = db.conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT profile_json FROM roster_cache
              WHERE team_id = ?1 AND cached_at >= ?2",
        )
        .map_err(|e| format!("Database error: {e}"))?;
    let rows: Vec<String> = stmt
        .query_map(params![team_id, now - ROSTER_TTL_MS], |row| row.get(0))
        .map_err(|e| format!("Database error: {e}"))?
        .collect::<rusqlite::Result<_>>()
        .map_err(|e| format!("Database error: {e}"))?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut members = Vec::new();
    for json in rows {
        match serde_json::from_str::<RosterMember>(&json) {
            Ok(member) => members.push(member),
            // An older shape: rebuild
            Err(_) => return Ok(None),
        }
    }
    sort_roster(&mut members);
    Ok(Some(members))
}

fn cache_roster(db: &Db, team_id: &str, members: &[RosterMember], now: i64) -> Result<(), String> {
    let mut conn = db.conn()?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| format!("Database error: {e}"))?;
    tx.execute("DELETE FROM roster_cache WHERE team_id = ?1", [team_id])
        .map_err(|e| format!("Database error: {e}"))?;
    for member in members {
        let json = serde_json::to_string(member).map_err(|e| format!("Encoding failed: {e}"))?;
        tx.execute(
            "INSERT OR REPLACE INTO roster_cache (team_id, member_handle, profile_json, cached_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![team_id, member.handle, json, now],
        )
        .map_err(|e| format!("Database error: {e}"))?;
    }
    tx.commit().map_err(|e| format!("Database error: {e}"))
}

/// Drops one team's cached roster, or every team's
pub fn clear_cache(db: &Db, team_id: Option<&str>) -> Result<(), String> {
    let conn = db.conn()?;
    match team_id {
        Some(team_id) => conn.execute("DELETE FROM roster_cache WHERE team_id = ?1", [team_id]),
        None => conn.execute("DELETE FROM roster_cache", []),
    }
    .map_err(|e| format!("Database error: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Building (pure, tested directly)
// ---------------------------------------------------------------------------

fn same_handle(a: &str, b: &str) -> bool {
    !a.is_empty() && a.trim().eq_ignore_ascii_case(b.trim())
}

/// The directory entry for a member: by an instance their cards came from,
/// else by handle; an online entry wins
fn find_member<'a>(
    snapshot: &'a NetworkSnapshot,
    handle: &str,
    instances: &HashSet<String>,
) -> Option<&'a NetworkMember> {
    let by_instance = snapshot
        .members
        .iter()
        .filter(|m| instances.contains(&m.instance_id));
    let by_handle = snapshot
        .members
        .iter()
        .filter(|m| m.handle.as_deref().is_some_and(|h| same_handle(h, handle)));
    let mut candidates: Vec<&NetworkMember> = by_instance.chain(by_handle).collect();
    // Members are sorted online first already; keep that order stable
    candidates.sort_by_key(|m| !m.online);
    candidates.into_iter().next()
}

fn sort_roster(members: &mut [RosterMember]) {
    members.sort_by(|a, b| {
        b.online
            .cmp(&a.online)
            .then_with(|| {
                a.display_name
                    .to_lowercase()
                    .cmp(&b.display_name.to_lowercase())
            })
            .then_with(|| a.handle.cmp(&b.handle))
    });
}

/// `team`'s members (its member list plus whoever published to its
/// registry), each with their gateway, what they shared to this team, and
/// how many of `teams` they're in. `stats` gives the delegation stats per
/// instance.
pub fn build_roster(
    team: &Team,
    teams: &[Team],
    cards: &[AgentCard],
    snapshot: &NetworkSnapshot,
    stats: &dyn Fn(&str) -> DelegationStats,
) -> Vec<RosterMember> {
    // handle → display name, in first-seen order
    let mut handles: BTreeMap<String, (String, String)> = BTreeMap::new();
    for member in &team.members {
        let key = member.github_handle.trim().to_lowercase();
        if key.is_empty() {
            continue;
        }
        handles.entry(key).or_insert((
            member.github_handle.trim().to_string(),
            member.display_name.clone(),
        ));
    }
    for card in cards {
        let key = card.owner.github_handle.trim().to_lowercase();
        if key.is_empty() {
            continue;
        }
        handles.entry(key).or_insert((
            card.owner.github_handle.trim().to_string(),
            card.owner.display_name.clone(),
        ));
    }

    let mut members: Vec<RosterMember> = handles
        .into_values()
        .map(|(handle, display_name)| {
            let shared: Vec<AgentCard> = cards
                .iter()
                .filter(|card| same_handle(&card.owner.github_handle, &handle))
                .cloned()
                .collect();
            let instances: HashSet<String> =
                shared.iter().map(|card| card.instance_id.clone()).collect();
            let member = find_member(snapshot, &handle, &instances);
            let instance_id = member
                .map(|m| m.instance_id.clone())
                .or_else(|| shared.first().map(|card| card.instance_id.clone()))
                .filter(|id| !id.is_empty());
            let team_count = teams
                .iter()
                .filter(|t| {
                    t.id == team.id
                        || t.members
                            .iter()
                            .any(|m| same_handle(&m.github_handle, &handle))
                })
                .count();
            let display_name = [
                display_name.as_str(),
                member.map_or("", |m| &m.display_name),
            ]
            .into_iter()
            .map(str::trim)
            .find(|name| !name.is_empty())
            .unwrap_or(&handle)
            .to_string();
            RosterMember {
                delegation_stats: instance_id.as_deref().map(stats).unwrap_or_default(),
                display_name,
                online: member.is_some_and(|m| m.online),
                gateway: member.map(|m| m.gateway.clone()),
                instance_id,
                shared_agents: shared,
                shared_workflows: Vec::new(),
                team_count: u32::try_from(team_count).unwrap_or(u32::MAX),
                handle,
            }
        })
        .collect();
    sort_roster(&mut members);
    members
}

/// Everything known about one instance, from what it shares only
#[allow(clippy::too_many_arguments)]
pub fn build_profile(
    instance_id: Option<&str>,
    handle_hint: Option<&str>,
    display_hint: Option<&str>,
    member: Option<&NetworkMember>,
    team_cards: Vec<(String, AgentCard)>,
    direct_cards: Vec<AgentCard>,
    trust: TrustRelationship,
    delegation_stats: DelegationStats,
    recent_delegations: Vec<TaskRecord>,
) -> MemberProfile {
    let blocked: HashSet<&str> = trust.blocked_agents.iter().map(String::as_str).collect();
    let mut seen = HashSet::new();
    let mut agents = Vec::new();
    for (team_id, card) in team_cards {
        if !seen.insert(card.id.clone()) {
            continue;
        }
        agents.push(MemberAgent {
            blocked: blocked.contains(card.agent_id()),
            card,
            source: MemberAgentSource::Team,
            team_id: Some(team_id),
        });
    }
    for card in direct_cards {
        if !seen.insert(card.id.clone()) {
            continue;
        }
        agents.push(MemberAgent {
            blocked: blocked.contains(card.agent_id()),
            card,
            source: MemberAgentSource::Direct,
            team_id: None,
        });
    }
    agents.sort_by(|a, b| {
        (a.source == MemberAgentSource::Direct)
            .cmp(&(b.source == MemberAgentSource::Direct))
            .then_with(|| a.card.name.to_lowercase().cmp(&b.card.name.to_lowercase()))
            .then_with(|| a.card.id.cmp(&b.card.id))
    });

    let card_owner = agents
        .iter()
        .map(|a| &a.card.owner)
        .find(|owner| !owner.github_handle.is_empty());
    let handle = member
        .and_then(|m| m.handle.clone())
        .or_else(|| handle_hint.map(str::to_string))
        .or_else(|| card_owner.map(|o| o.github_handle.clone()))
        .filter(|h| !h.trim().is_empty());
    let display_name = [
        member.map(|m| m.display_name.as_str()),
        display_hint,
        card_owner.map(|o| o.display_name.as_str()),
        handle.as_deref(),
        instance_id,
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
        instance_id: instance_id.map(str::to_string),
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

fn manager(state: &AppState) -> Result<&dyn TeamManager, String> {
    state
        .team_manager()
        .map(|manager| manager.as_ref())
        .ok_or_else(|| "Teams aren't available".to_string())
}

/// The directory, or an empty one when it can't be read
async fn directory(state: &AppState) -> NetworkSnapshot {
    network::get_network_snapshot(state)
        .await
        .unwrap_or_else(|e| {
            log::debug!("Roster: no network directory: {e}");
            NetworkSnapshot {
                relay_url: None,
                relay_connected: false,
                members: Vec::new(),
                fetched_at: now_ms() as Timestamp,
            }
        })
}

pub async fn get_team_roster(state: &AppState, team_id: &str) -> Result<TeamRoster, String> {
    let now = now_ms();
    if let Some(members) = cached_roster(state.db(), team_id, now)? {
        return Ok(TeamRoster {
            team_id: team_id.to_string(),
            members,
            fetched_at: now as Timestamp,
        });
    }
    let manager = manager(state)?;
    let teams = manager.list_teams().await?;
    let team = teams
        .iter()
        .find(|team| team.id == team_id)
        .ok_or_else(|| format!("Team {team_id} not found"))?;
    let cards = manager.team_agents(team_id).await.unwrap_or_else(|e| {
        log::debug!("Roster: team {team_id} registry unavailable: {e}");
        Vec::new()
    });
    let snapshot = directory(state).await;
    let local = network::local_instance_id(state.db())?;
    let db = state.db().clone();
    let stats = move |theirs: &str| delegation_stats(&db, &local, theirs).unwrap_or_default();
    let members = build_roster(team, &teams, &cards, &snapshot, &stats);
    if let Err(e) = cache_roster(state.db(), team_id, &members, now) {
        log::debug!("Roster cache: {e}");
    }
    Ok(TeamRoster {
        team_id: team_id.to_string(),
        members,
        fetched_at: now as Timestamp,
    })
}

/// A member of `team_id` by handle: their instance (from the directory or
/// their shared cards), else a not-yet-connected profile
pub async fn get_member_profile(
    state: &AppState,
    team_id: &str,
    handle: &str,
) -> Result<MemberProfile, String> {
    let handle = handle.trim();
    if handle.is_empty() {
        return Err("Choose a member".to_string());
    }
    let manager = manager(state)?;
    let teams = manager.list_teams().await?;
    let team = teams
        .iter()
        .find(|team| team.id == team_id)
        .ok_or_else(|| format!("Team {team_id} not found"))?;
    let display_hint = team
        .members
        .iter()
        .find(|m| same_handle(&m.github_handle, handle))
        .map(|m| m.display_name.clone());
    let cards = manager.team_agents(team_id).await.unwrap_or_default();
    let instances: HashSet<String> = cards
        .iter()
        .filter(|card| same_handle(&card.owner.github_handle, handle))
        .map(|card| card.instance_id.clone())
        .collect();
    let snapshot = directory(state).await;
    let instance = find_member(&snapshot, handle, &instances)
        .map(|m| m.instance_id.clone())
        .or_else(|| instances.into_iter().find(|id| !id.is_empty()));
    profile(
        state,
        instance.as_deref(),
        Some(handle),
        display_hint.as_deref(),
        &snapshot,
    )
    .await
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
    profile(state, Some(instance_id), None, None, &snapshot).await
}

async fn profile(
    state: &AppState,
    instance_id: Option<&str>,
    handle: Option<&str>,
    display_hint: Option<&str>,
    snapshot: &NetworkSnapshot,
) -> Result<MemberProfile, String> {
    let Some(instance_id) = instance_id else {
        // Never connected and shared nothing: the empty state
        return Ok(build_profile(
            None,
            handle,
            display_hint,
            None,
            Vec::new(),
            Vec::new(),
            TrustRelationship::default(),
            DelegationStats::default(),
            Vec::new(),
        ));
    };
    let member = snapshot
        .members
        .iter()
        .find(|m| m.instance_id == instance_id);
    let team_cards: Vec<(String, AgentCard)> = match state.team_manager() {
        Some(manager) => crate::services::shared::team_cards(manager.as_ref())
            .await
            .into_iter()
            .filter(|(_, card)| card.instance_id == instance_id)
            .collect(),
        None => Vec::new(),
    };
    let direct: Vec<AgentCard> = relay::list_online_agents(state, None)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|card| card.instance_id == instance_id)
        .collect();
    let local = network::local_instance_id(state.db())?;
    let trust = trust::get_relationship(state.db(), instance_id)?;
    let stats = delegation_stats(state.db(), &local, instance_id)?;
    let recent = recent_delegations(state.db(), &local, instance_id, RECENT_DELEGATIONS)?;
    let mut profile = build_profile(
        Some(instance_id),
        handle,
        display_hint,
        member,
        team_cards,
        direct,
        trust,
        stats,
        recent,
    );
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
    use crate::services::gateway::GatewayCard;
    use crate::services::team::{MemberRole, TeamMember, TeamSource};

    fn card(instance: &str, agent: &str, handle: &str) -> AgentCard {
        let skill = format!("---\nname: {agent}\nserves: [{{name: find-papers}}]\n---\n");
        let def = Agent::from_skill_md(AgentId::parse(agent).unwrap(), &skill).unwrap();
        card_from_agent(&def, instance, "").with_owner(OwnerRef {
            github_handle: handle.to_string(),
            display_name: format!("{handle} name"),
        })
    }

    fn team(id: &str, handles: &[&str]) -> Team {
        Team {
            id: id.to_string(),
            name: id.to_string(),
            github_org: id.to_string(),
            registry_repo: String::new(),
            members: handles
                .iter()
                .map(|h| TeamMember {
                    github_handle: h.to_string(),
                    display_name: format!("{h} (team)"),
                    role: MemberRole::Member,
                    joined_at: 0.0,
                })
                .collect(),
            created_at: 0.0,
            source: TeamSource::git("https://example.com/r.git"),
        }
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

    fn snapshot(members: Vec<NetworkMember>) -> NetworkSnapshot {
        NetworkSnapshot {
            relay_url: None,
            relay_connected: true,
            members,
            fetched_at: 0.0,
        }
    }

    #[test]
    fn rosters_join_members_publishers_gateways_and_teams() {
        let acme = team("acme", &["alice", "bob"]);
        let beta = team("beta", &["Alice"]);
        let cards = vec![
            card("i-alice", "papers", "alice"),
            card("i-carol", "notes", "carol"),
        ];
        let dir = snapshot(vec![member("i-bob", "bob", true)]);
        let stats = |instance: &str| DelegationStats {
            sent: if instance == "i-alice" { 2 } else { 0 },
            ..DelegationStats::default()
        };
        let roster = build_roster(&acme, &[acme.clone(), beta], &cards, &dir, &stats);
        let handles: Vec<&str> = roster.iter().map(|m| m.handle.as_str()).collect();
        // Online first, then by name; carol only published
        assert_eq!(handles, ["bob", "alice", "carol"]);
        let bob = &roster[0];
        assert!(bob.online && bob.gateway.is_some());
        assert_eq!(bob.instance_id.as_deref(), Some("i-bob"));
        assert!(bob.shared_agents.is_empty());
        let alice = &roster[1];
        assert!(!alice.online && alice.gateway.is_none());
        assert_eq!(alice.instance_id.as_deref(), Some("i-alice"));
        assert_eq!(alice.shared_agents.len(), 1);
        assert_eq!(alice.team_count, 2);
        assert_eq!(alice.delegation_stats.sent, 2);
        assert_eq!(alice.display_name, "alice (team)");
        let carol = &roster[2];
        assert_eq!(carol.team_count, 1);
        assert_eq!(carol.display_name, "carol name");
        assert!(carol.shared_workflows.is_empty());
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
    fn profiles_group_agents_and_mark_blocked_ones() {
        let trust = TrustRelationship {
            instance_id: "i-a".into(),
            blocked_agents: vec!["notes".into()],
            ..TrustRelationship::default()
        };
        let profile = build_profile(
            Some("i-a"),
            None,
            None,
            Some(&member("i-a", "alice", true)),
            vec![
                ("acme".into(), card("i-a", "papers", "alice")),
                ("beta".into(), card("i-a", "papers", "alice")),
            ],
            vec![
                card("i-a", "papers", "alice"),
                card("i-a", "notes", "alice"),
            ],
            trust,
            DelegationStats::default(),
            Vec::new(),
        );
        assert!(profile.connected && profile.online);
        assert_eq!(profile.handle.as_deref(), Some("alice"));
        assert_eq!(profile.capabilities, ["summarize"]);
        let agents: Vec<(&str, MemberAgentSource, bool)> = profile
            .agents
            .iter()
            .map(|a| (a.card.id.as_str(), a.source, a.blocked))
            .collect();
        assert_eq!(
            agents,
            [
                ("i-a/papers", MemberAgentSource::Team, false),
                ("i-a/notes", MemberAgentSource::Direct, true),
            ]
        );
        assert_eq!(profile.agents[0].team_id.as_deref(), Some("acme"));
    }

    #[test]
    fn a_member_without_a_gateway_is_not_connected() {
        let profile = build_profile(
            None,
            Some("dana"),
            Some("Dana"),
            None,
            Vec::new(),
            Vec::new(),
            TrustRelationship::default(),
            DelegationStats::default(),
            Vec::new(),
        );
        assert!(!profile.connected && !profile.online);
        assert!(profile.gateway.is_none() && profile.agents.is_empty());
        assert_eq!(profile.display_name, "Dana");
        assert_eq!(profile.handle.as_deref(), Some("dana"));

        // Known only by shared cards: capabilities come from them
        let profile = build_profile(
            Some("i-c"),
            None,
            None,
            None,
            vec![("acme".into(), card("i-c", "papers", "carol"))],
            Vec::new(),
            TrustRelationship::default(),
            DelegationStats::default(),
            Vec::new(),
        );
        assert!(!profile.connected);
        assert_eq!(profile.capabilities, ["find-papers"]);
        assert_eq!(profile.display_name, "carol name");
    }

    #[test]
    fn rosters_are_cached_for_a_minute() {
        let db = Db::in_memory();
        assert!(cached_roster(&db, "acme", 0).unwrap().is_none());
        let acme = team("acme", &["alice"]);
        let roster = build_roster(&acme, &[], &[], &snapshot(Vec::new()), &|_| {
            DelegationStats::default()
        });
        cache_roster(&db, "acme", &roster, 1_000).unwrap();
        assert_eq!(cached_roster(&db, "acme", 30_000).unwrap().unwrap(), roster);
        assert!(cached_roster(&db, "acme", 1_000 + ROSTER_TTL_MS + 1)
            .unwrap()
            .is_none());
        clear_cache(&db, Some("acme")).unwrap();
        assert!(cached_roster(&db, "acme", 30_000).unwrap().is_none());
    }

    #[tokio::test]
    async fn an_unknown_instance_gets_the_empty_state() {
        let state = AppState::for_tests(vec![]);
        assert!(state.team_roster().is_some());
        let profile = get_member_by_instance(&state, "i-nobody").await.unwrap();
        assert!(!profile.connected);
        assert_eq!(profile.instance_id.as_deref(), Some("i-nobody"));
        assert_eq!(profile.display_name, "i-nobody");
        assert_eq!(profile.trust.instance_id, "i-nobody");
        assert!(get_member_by_instance(&state, " ").await.is_err());
        state.shutdown();
    }
}
