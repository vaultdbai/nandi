//! Wave 2A wiring: teams, cross-user agent sharing, and delegation.
//!
//! FROZEN. Each track fills in its own module's `init`; `AppState` calls
//! them in dependency order when it starts:
//!
//! 1. `capability_index::init` (Track A)
//! 2. `relay::init` (Track B)
//! 3. `team::init` (Track D)
//! 4. `a2a::init` (Track C), which may use the three above
//!
//! An `init` returns `None` until its track lands. It must not block or
//! touch the network on the calling thread: connecting happens in tasks it
//! spawns, and only when the user configured it.

// Wave EF: member profiles by instance
pub mod peer_keys;
pub mod profiles;

use std::sync::{Arc, RwLock};

use rusqlite::{params, OptionalExtension};

use super::a2a::{self, A2ADelegator};
use super::capability_index::{self, CapabilityIndex};
use super::relay::{self, RelayClient};
use super::team::{self, TeamManager};
use super::AppState;
use crate::db::Db;

/// `relay_state` key holding this install's instance ID
pub const INSTANCE_ID_KEY: &str = "instance_id";

/// The network services that `init` produced (each `None` until its track
/// lands).
#[derive(Default)]
pub struct NetworkServices {
    pub capability_index: Option<Arc<RwLock<CapabilityIndex>>>,
    pub relay_client: Option<Arc<dyn RelayClient>>,
    pub team_manager: Option<Arc<dyn TeamManager>>,
    pub a2a_delegator: Option<Arc<dyn A2ADelegator>>,
}

/// Runs each track's `init`, in dependency order, storing each result in
/// `AppState` before the next one runs.
pub(super) fn init(state: &AppState) {
    if let Some(index) = capability_index::init(state) {
        state.set_capability_index(index);
    }
    if let Some(client) = relay::init(state) {
        state.workflows().use_relay(client.clone());
        super::groups::watch(state, &client);
        state.set_relay_client(client);
    }
    if let Some(manager) = team::init(state) {
        state.set_team_manager(manager);
    }
    if let Some(delegator) = a2a::init(state) {
        state.set_a2a_delegator(delegator);
    }
}

/// This install's stable instance ID, created on first use as
/// `<hostname>-<8 hex>` (e.g. `alice-laptop-1a2b3c4d`; installs from before
/// wave 2C keep their 32-hex ID). Cards are `<instance_id>/<agent_id>`.
pub fn local_instance_id(db: &Db) -> Result<String, String> {
    let conn = db.conn()?;
    let existing: Option<String> = conn
        .query_row(
            "SELECT value FROM relay_state WHERE key = ?1",
            [INSTANCE_ID_KEY],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("Database error: {e}"))?;
    if let Some(value) = existing {
        // Stored as JSON: a quoted string
        if let Ok(id) = serde_json::from_str::<String>(&value) {
            return Ok(id);
        }
    }
    let id = new_instance_id(&host_label());
    conn.execute(
        "INSERT INTO relay_state (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![
            INSTANCE_ID_KEY,
            serde_json::to_string(&id).unwrap_or_default()
        ],
    )
    .map_err(|e| format!("Database error: {e}"))?;
    Ok(id)
}

/// `~/.nandi/local/instance_id`: the instance ID on disk, so rebuilding or
/// deleting `state.db` keeps this install's identity on the network (peers'
/// trust, caller policies, task history and pins all name it)
pub const INSTANCE_ID_FILE: &str = "instance_id";

/// Keeps the instance ID in `local_dir` and the database the same: the
/// file wins when it holds one; otherwise the database's (created if
/// needed) is written to it. Failures are logged; the database's ID still
/// works for this run.
pub fn keep_instance_id(db: &Db, local_dir: &std::path::Path) {
    let path = local_dir.join(INSTANCE_ID_FILE);
    let on_disk = std::fs::read_to_string(&path)
        .ok()
        .map(|id| id.trim().to_string())
        .filter(|id| is_instance_id(id));
    let result = match on_disk {
        Some(id) => set_instance_id(db, &id),
        None => local_instance_id(db).and_then(|id| {
            std::fs::create_dir_all(local_dir)
                .and_then(|()| {
                    std::fs::write(
                        &path,
                        format!(
                            "{id}
"
                        ),
                    )
                })
                .map_err(|e| format!("Failed to write {}: {e}", path.display()))
        }),
    };
    if let Err(e) = result {
        log::warn!("Instance ID: {e}");
    }
}

/// A plausible instance ID: not empty, no whitespace or `/` (card IDs are
/// `<instance>/<agent>`)
fn is_instance_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && !id.contains('/') && !id.contains(char::is_whitespace)
}

fn set_instance_id(db: &Db, id: &str) -> Result<(), String> {
    db.conn()?
        .execute(
            "INSERT INTO relay_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![
                INSTANCE_ID_KEY,
                serde_json::to_string(id).unwrap_or_default()
            ],
        )
        .map_err(|e| format!("Database error: {e}"))?;
    Ok(())
}

/// `<label>-<8 random hex>`: readable in relay logs, unique per install
fn new_instance_id(label: &str) -> String {
    let random = uuid::Uuid::new_v4().simple().to_string();
    format!("{label}-{}", &random[..8])
}

/// This machine's name as an instance ID prefix (`nandi` if unknown)
fn host_label() -> String {
    let name = ["COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|var| std::env::var(var).ok().filter(|v| !v.trim().is_empty()))
        .or_else(|| {
            std::process::Command::new("hostname")
                .output()
                .ok()
                .and_then(|out| String::from_utf8(out.stdout).ok())
        })
        .unwrap_or_default();
    sanitize_label(&name)
}

/// Lowercase letters, digits, and single dashes, at most 24 characters,
/// without a domain suffix; `nandi` when nothing is left
fn sanitize_label(name: &str) -> String {
    let name = name.trim().split('.').next().unwrap_or_default();
    let mut label = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            label.push(c);
        } else if !label.is_empty() && !label.ends_with('-') {
            label.push('-');
        }
    }
    let label: String = label.chars().take(24).collect();
    let label = label.trim_end_matches('-');
    if label.is_empty() {
        "nandi".to_string()
    } else {
        label.to_string()
    }
}

/// Where the relay at `relay_url` reaches a local agent:
/// `wss://relay.example/a/<instance>/<agent>`.
pub fn agent_endpoint(relay_url: &str, instance_id: &str, agent_id: &str) -> String {
    let base = relay_url.trim_end_matches('/');
    let base = base.strip_suffix("/relay").unwrap_or(base);
    format!("{base}/a/{instance_id}/{agent_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_instance_id_survives_a_new_database() {
        let local = tempfile::tempdir().unwrap();
        let first = Db::in_memory();
        keep_instance_id(&first, local.path());
        let id = local_instance_id(&first).unwrap();
        let on_disk = std::fs::read_to_string(local.path().join(INSTANCE_ID_FILE)).unwrap();
        assert_eq!(on_disk.trim(), id);

        // state.db deleted and rebuilt: same identity
        let rebuilt = Db::in_memory();
        keep_instance_id(&rebuilt, local.path());
        assert_eq!(local_instance_id(&rebuilt).unwrap(), id);

        // A broken file is replaced by the database's ID
        std::fs::write(local.path().join(INSTANCE_ID_FILE), "bad id/x").unwrap();
        let other = Db::in_memory();
        keep_instance_id(&other, local.path());
        let fresh = local_instance_id(&other).unwrap();
        let on_disk = std::fs::read_to_string(local.path().join(INSTANCE_ID_FILE)).unwrap();
        assert_eq!(on_disk.trim(), fresh);
    }

    #[test]
    fn instance_id_is_created_once() {
        let db = Db::in_memory();
        let first = local_instance_id(&db).unwrap();
        assert_eq!(local_instance_id(&db).unwrap(), first);
        let (label, random) = first.rsplit_once('-').unwrap();
        assert!(!label.is_empty() && !first.contains('/'));
        assert_eq!(random.len(), 8);
        assert!(random.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn instance_ids_start_with_a_clean_host_label() {
        assert_eq!(
            sanitize_label("Alice's MacBook Pro.local"),
            "alice-s-macbook-pro"
        );
        assert_eq!(sanitize_label("DESKTOP-7Q2X"), "desktop-7q2x");
        assert_eq!(sanitize_label("  "), "nandi");
        assert_eq!(sanitize_label("a/b"), "a-b");
        assert_eq!(sanitize_label(&"x".repeat(40)).len(), 24);
        let id = new_instance_id("alice-laptop");
        assert!(id.starts_with("alice-laptop-"));
        assert_eq!(id.len(), "alice-laptop-".len() + 8);
    }

    #[test]
    fn endpoints_hang_off_the_relay() {
        assert_eq!(
            agent_endpoint("wss://relay.example/relay", "i1", "papers"),
            "wss://relay.example/a/i1/papers"
        );
        assert_eq!(
            agent_endpoint("wss://relay.example/", "i1", "papers"),
            "wss://relay.example/a/i1/papers"
        );
    }

    #[tokio::test]
    async fn init_runs_without_a_relay_configured() {
        // Each slot is `None` until its track lands; either way, starting
        // must not need the network
        let state = AppState::for_tests(vec![]);
        let _ = (
            state.capability_index().is_some(),
            state.relay_client().is_some(),
            state.a2a_delegator().is_some(),
            state.team_manager().is_some(),
        );
        state.shutdown();
    }
}

// ---------------------------------------------------------------------------
// The network directory: everyone on the relay, from the relay's member
// directory (`QueryDirectory`), which keeps offline members until they're
// revoked. Only gateway cards and explicitly shared items are ever shown,
// never a member's private data. With the relay down the last listing is
// shown, every member offline and marked as last seen; refreshing then
// answers `RelayUnavailable`.
// ---------------------------------------------------------------------------

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use specta::Type;

use super::events::{emit, NandiEvent};
use super::gateway::GatewayCard;
use super::relay::protocol::MemberRecord;
use super::relay::RelayUnavailable;
use super::team::roster::MEMBER_STATUS_CHANGED;
use crate::model::Timestamp;

/// `network://member_online`
pub const NETWORK_MEMBER_ONLINE: &str = "network://member_online";
/// `network://member_offline`
pub const NETWORK_MEMBER_OFFLINE: &str = "network://member_offline";
/// `network://membership_changed`
pub const NETWORK_MEMBERSHIP_CHANGED: &str = "network://membership_changed";
/// `network://member_joined`: an instance seen for the first time
pub const NETWORK_MEMBER_JOINED: &str = "network://member_joined";
/// `network://member_left`: a member went offline
pub const NETWORK_MEMBER_LEFT: &str = "network://member_left";
/// `network://profile_updated`: a member's gateway card changed
pub const NETWORK_PROFILE_UPDATED: &str = "network://profile_updated";

/// One instance on the relay. Duplicate names are allowed: `instance_id`
/// is the identity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NetworkMember {
    pub instance_id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub online: bool,
    /// Epoch ms
    pub last_seen: Option<Timestamp>,
    pub gateway: GatewayCard,
    /// The owner's handle (user slug), when the card names one
    pub handle: Option<String>,
}

/// Online first, then by display name
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSnapshot {
    pub relay_url: Option<String>,
    pub relay_connected: bool,
    pub members: Vec<NetworkMember>,
    pub fetched_at: Timestamp,
}

/// How long a snapshot is served from the cache
pub const SNAPSHOT_TTL: Duration = Duration::from_secs(10);
/// How long the relay gets to answer `QueryDirectory`
pub const QUERY_TIMEOUT: Duration = Duration::from_secs(5);
/// `relay_state` key holding every member seen so far (JSON list), so
/// offline members and their last-seen times survive a restart
pub const KNOWN_MEMBERS_KEY: &str = "network_members";

/// Payload of `network://member_online` / `network://member_offline` (and
/// `member://status_changed`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NetworkMemberChanged {
    pub member: NetworkMember,
}

/// Payload of `network://membership_changed`: someone new appeared
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NetworkMembershipChanged {
    pub member_count: u32,
}

impl NandiEvent for NetworkMembershipChanged {
    const NAME: &'static str = NETWORK_MEMBERSHIP_CHANGED;
}

/// The event names share one payload type, so each gets a wrapper
struct MemberOnline<'a>(&'a NetworkMemberChanged);
struct MemberOffline<'a>(&'a NetworkMemberChanged);
struct MemberStatus<'a>(&'a NetworkMemberChanged);
struct MemberJoined<'a>(&'a NetworkMemberChanged);
struct MemberLeft<'a>(&'a NetworkMemberChanged);
struct ProfileUpdated<'a>(&'a NetworkMemberChanged);

macro_rules! member_event {
    ($ty:ident, $name:expr) => {
        impl Serialize for $ty<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                self.0.serialize(s)
            }
        }
        impl NandiEvent for $ty<'_> {
            const NAME: &'static str = $name;
        }
    };
}
member_event!(MemberOnline, NETWORK_MEMBER_ONLINE);
member_event!(MemberOffline, NETWORK_MEMBER_OFFLINE);
member_event!(MemberStatus, MEMBER_STATUS_CHANGED);
member_event!(MemberJoined, NETWORK_MEMBER_JOINED);
member_event!(MemberLeft, NETWORK_MEMBER_LEFT);
member_event!(ProfileUpdated, NETWORK_PROFILE_UPDATED);

/// What one refresh changed, for the events
#[derive(Debug, Default, PartialEq)]
pub struct DirectoryChanges {
    pub came_online: Vec<NetworkMember>,
    pub went_offline: Vec<NetworkMember>,
    /// An instance we hadn't seen before appeared
    pub membership_changed: bool,
    /// Instances seen for the first time (wave EF; also in `came_online`)
    pub joined: Vec<NetworkMember>,
    /// Known instances whose gateway card changed (wave EF)
    pub profile_updated: Vec<NetworkMember>,
}

impl DirectoryChanges {
    pub fn is_empty(&self) -> bool {
        self.came_online.is_empty()
            && self.went_offline.is_empty()
            && !self.membership_changed
            && self.profile_updated.is_empty()
    }
}

/// The directory: the relay's members (online or not), and while the relay
/// can't be asked, the last listing with everyone offline. Cached for 10s.
pub struct NetworkService {
    /// The last snapshot and when it was built
    cache: Mutex<Option<(Instant, NetworkSnapshot)>>,
    /// Every member seen, by instance ID
    known: Mutex<HashMap<String, NetworkMember>>,
    /// One refresh at a time (relay events can arrive in bursts)
    refreshing: tokio::sync::Mutex<()>,
}

impl NetworkService {
    /// Starts from the members stored in `relay_state`, all offline until
    /// the relay says otherwise
    pub fn new(db: &Db) -> Self {
        let known = load_known(db)
            .into_iter()
            .map(|mut member| {
                member.online = false;
                (member.instance_id.clone(), member)
            })
            .collect();
        Self {
            cache: Mutex::new(None),
            known: Mutex::new(known),
            refreshing: tokio::sync::Mutex::new(()),
        }
    }

    /// The cached snapshot, if younger than `SNAPSHOT_TTL`
    pub fn cached(&self) -> Option<NetworkSnapshot> {
        let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        cache
            .as_ref()
            .filter(|(at, _)| at.elapsed() < SNAPSHOT_TTL)
            .map(|(_, snapshot)| snapshot.clone())
    }

    /// Drops the cache so the next read asks the relay
    pub fn invalidate(&self) {
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Asks the relay for its directory (5s timeout) and takes it as the
    /// known members. On error (or without a relay), every known member is
    /// offline. Persists the known members.
    pub async fn refresh_with(
        &self,
        db: &Db,
        relay: Option<&dyn RelayClient>,
        local_instance: &str,
    ) -> (NetworkSnapshot, DirectoryChanges) {
        let _guard = self.refreshing.lock().await;
        let status = relay.map(|client| client.status());
        // Without this instance's token: the URL goes into invite messages
        let relay_url = status
            .as_ref()
            .and_then(|s| s.relay_url.as_deref())
            .map(|url| nandi_protocol::split_token(url).0);
        let relay_connected = status.as_ref().is_some_and(|s| s.connected);

        let records = match relay {
            Some(client) if relay_connected => {
                match tokio::time::timeout(QUERY_TIMEOUT, client.query_directory()).await {
                    Ok(Ok(records)) => Some(records),
                    Ok(Err(e)) => {
                        log::debug!("Network directory: relay query failed: {e}");
                        None
                    }
                    Err(_) => {
                        log::debug!("Network directory: relay query timed out");
                        None
                    }
                }
            }
            _ => None,
        };
        if let Some(records) = &records {
            for id in peer_keys::observe_directory(db, records, local_instance) {
                log::warn!("{id}'s key changed; sending to them waits until it's accepted");
            }
        }

        let now = now_ms() as Timestamp;
        let (members, changes) = {
            let mut known = self.known.lock().unwrap_or_else(|e| e.into_inner());
            let changes = match records {
                Some(records) => merge_directory(&mut known, records, local_instance),
                None => mark_offline(&mut known),
            };
            let mut members: Vec<NetworkMember> = known.values().cloned().collect();
            sort_members(&mut members);
            (members, changes)
        };
        if let Err(e) = save_known(db, &members) {
            log::warn!("Failed to save the network members: {e}");
        }
        let snapshot = NetworkSnapshot {
            relay_url,
            relay_connected,
            members,
            fetched_at: now,
        };
        *self.cache.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((Instant::now(), snapshot.clone()));
        (snapshot, changes)
    }
}

/// Takes the relay's directory as `known`: its members, online or not,
/// with the relay's last-seen times. Members it no longer lists (revoked)
/// are dropped. Our own instance is left out.
pub fn merge_directory(
    known: &mut HashMap<String, NetworkMember>,
    records: Vec<MemberRecord>,
    local_instance: &str,
) -> DirectoryChanges {
    let mut changes = DirectoryChanges::default();
    let mut listed = HashMap::new();
    for record in records {
        let id = record.card.instance_id.clone();
        if id.is_empty() || id == local_instance || listed.contains_key(&id) {
            continue;
        }
        let member = member_from_record(record);
        match known.get(&id) {
            None => {
                changes.membership_changed = true;
                changes.joined.push(member.clone());
                if member.online {
                    changes.came_online.push(member.clone());
                }
            }
            Some(previous) => {
                if previous.gateway != member.gateway {
                    changes.profile_updated.push(member.clone());
                }
                match (previous.online, member.online) {
                    (false, true) => changes.came_online.push(member.clone()),
                    (true, false) => changes.went_offline.push(member.clone()),
                    _ => {}
                }
            }
        }
        listed.insert(id, member);
    }
    *known = listed;
    changes
}

/// The relay can't be asked: every known member is offline, keeping their
/// last-seen time
pub fn mark_offline(known: &mut HashMap<String, NetworkMember>) -> DirectoryChanges {
    let mut changes = DirectoryChanges::default();
    for member in known.values_mut().filter(|member| member.online) {
        member.online = false;
        changes.went_offline.push(member.clone());
    }
    changes
}

/// A member as the relay's directory lists them
pub fn member_from_record(record: MemberRecord) -> NetworkMember {
    let mut member = member_from_card(record.card, record.last_seen_ms);
    member.online = record.online;
    member.last_seen = (record.last_seen_ms > 0.0).then_some(record.last_seen_ms);
    member
}

/// A member as its gateway card describes it, online and seen `now`: the
/// card's display name (else the owner's, else the instance ID), the
/// owner's handle if any
pub fn member_from_card(card: GatewayCard, now: Timestamp) -> NetworkMember {
    let display_name = [&card.display_name, &card.owner.display_name]
        .into_iter()
        .map(|name| name.trim())
        .find(|name| !name.is_empty())
        .unwrap_or(&card.instance_id)
        .to_string();
    let handle = Some(card.owner.github_handle.trim().to_string()).filter(|h| !h.is_empty());
    NetworkMember {
        instance_id: card.instance_id.clone(),
        display_name,
        avatar_url: None,
        online: true,
        last_seen: Some(now),
        gateway: card,
        handle,
    }
}

/// Online first, then display name (case-insensitive), then instance ID
pub fn sort_members(members: &mut [NetworkMember]) {
    members.sort_by(|a, b| {
        b.online
            .cmp(&a.online)
            .then_with(|| {
                a.display_name
                    .to_lowercase()
                    .cmp(&b.display_name.to_lowercase())
            })
            .then_with(|| a.instance_id.cmp(&b.instance_id))
    });
}

/// The members as last seen (online or not), without asking the relay
pub fn known_members(db: &Db) -> Vec<NetworkMember> {
    load_known(db)
}

fn load_known(db: &Db) -> Vec<NetworkMember> {
    let value: Option<String> = db.conn().ok().and_then(|conn| {
        conn.query_row(
            "SELECT value FROM relay_state WHERE key = ?1",
            [KNOWN_MEMBERS_KEY],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
    });
    value
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

/// For tests elsewhere: members as if the relay had listed them
#[cfg(test)]
pub(crate) fn remember_members(db: &Db, members: &[NetworkMember]) {
    save_known(db, members).unwrap();
}

fn save_known(db: &Db, members: &[NetworkMember]) -> Result<(), String> {
    let json = serde_json::to_string(members).map_err(|e| format!("Encoding failed: {e}"))?;
    db.conn()?
        .execute(
            "INSERT INTO relay_state (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![KNOWN_MEMBERS_KEY, json],
        )
        .map_err(|e| format!("Database error: {e}"))?;
    Ok(())
}

fn now_ms() -> i64 {
    crate::db::now_ms()
}

/// Emits `network://*` (and `member://status_changed`) for what changed
fn emit_changes(state: &AppState, changes: &DirectoryChanges, member_count: usize) {
    let payload = |member: &NetworkMember| NetworkMemberChanged {
        member: member.clone(),
    };
    for member in &changes.joined {
        emit(state.bus(), &MemberJoined(&payload(member)));
    }
    for member in &changes.came_online {
        let payload = payload(member);
        emit(state.bus(), &MemberOnline(&payload));
        emit(state.bus(), &MemberStatus(&payload));
    }
    for member in &changes.went_offline {
        let payload = payload(member);
        emit(state.bus(), &MemberOffline(&payload));
        emit(state.bus(), &MemberLeft(&payload));
        emit(state.bus(), &MemberStatus(&payload));
    }
    for member in &changes.profile_updated {
        emit(state.bus(), &ProfileUpdated(&payload(member)));
    }
    if changes.membership_changed {
        emit(
            state.bus(),
            &NetworkMembershipChanged {
                member_count: u32::try_from(member_count).unwrap_or(u32::MAX),
            },
        );
    }
}

/// Builds the directory and, in the background, refreshes it whenever the
/// relay reports presence or connection changes (diffing for the
/// `network://*` events). Never touches the network on this thread.
pub fn init_directory(state: &AppState) -> Option<Arc<NetworkService>> {
    let service = Arc::new(NetworkService::new(state.db()));
    let mut events = state.broadcast().subscribe();
    // Weak, so the listener ends with the app (the sender lives in it)
    let weak = Arc::downgrade(&state.0);
    state.runtime().spawn(async move {
        use tokio::sync::broadcast::error::RecvError;
        loop {
            match events.recv().await {
                Ok(event)
                    if matches!(
                        event.event,
                        "relay://agent_online" | "relay://agent_offline" | "relay://status"
                    ) => {}
                Ok(_) | Err(RecvError::Lagged(_)) => continue,
                Err(RecvError::Closed) => break,
            }
            let Some(inner) = weak.upgrade() else { break };
            let state = AppState(inner);
            if let Err(e) = snapshot(&state, true).await {
                log::debug!("Network directory refresh failed: {e}");
            }
        }
    });
    Some(service)
}

async fn snapshot(state: &AppState, force: bool) -> Result<NetworkSnapshot, String> {
    let service = state
        .network()
        .ok_or_else(|| "The network directory isn't available".to_string())?;
    if !force {
        if let Some(snapshot) = service.cached() {
            return Ok(snapshot);
        }
    }
    let local = local_instance_id(state.db())?;
    let relay = state.relay_client().map(|client| client.as_ref());
    let (snapshot, changes) = service.refresh_with(state.db(), relay, &local).await;
    if !changes.is_empty() {
        emit_changes(state, &changes, snapshot.members.len());
    }
    Ok(snapshot)
}

/// Cached for 10s
pub async fn get_network_snapshot(state: &AppState) -> Result<NetworkSnapshot, String> {
    snapshot(state, false).await
}

/// Asks the relay again, bypassing the cache. Without a connected relay
/// the members are marked offline and the answer is `RelayUnavailable`
/// (the cached listing stays readable through `get_network_snapshot`).
pub async fn refresh_network(state: &AppState) -> Result<NetworkSnapshot, String> {
    let snapshot = snapshot(state, true).await?;
    if !snapshot.relay_connected {
        return Err(RelayUnavailable.into());
    }
    Ok(snapshot)
}

#[cfg(test)]
pub(crate) mod directory_tests {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use futures_util::future::BoxFuture;

    use super::*;
    use crate::services::a2a::{Task, TaskResult};
    use crate::services::domain::agent_card::OwnerRef;
    use crate::services::domain::AgentCard;
    use crate::services::relay::client_api::{Handler, PeopleSearch};
    use crate::services::relay::protocol::{TaskAssigned, TaskCompleted, TaskFailed};
    use crate::services::relay::RelayStatus;

    pub(crate) fn gateway(instance: &str, name: &str, handle: &str) -> GatewayCard {
        GatewayCard {
            instance_id: instance.to_string(),
            endpoint: format!("wss://relay/a/{instance}/_gateway"),
            display_name: name.to_string(),
            description: format!("{name}'s Nandi"),
            capabilities: vec!["find-papers".to_string()],
            accepts_delegation: true,
            max_concurrent: 3,
            current_load: 0,
            owner: OwnerRef {
                github_handle: handle.to_string(),
                display_name: name.to_string(),
            },
            federation_enabled: false,
            gateway_version: 1,
            agent_count: 2,
            body: None,
            direct_delegation: None,
            responsibilities: Vec::new(),
            availability: Default::default(),
        }
    }

    /// A member the relay lists as online
    pub(crate) fn online(card: GatewayCard) -> MemberRecord {
        MemberRecord {
            card,
            online: true,
            last_seen_ms: 1_000.0,
            public_keys: None,
        }
    }

    /// A member the relay lists as offline, last seen at `seen`
    pub(crate) fn offline(card: GatewayCard, seen: Timestamp) -> MemberRecord {
        MemberRecord {
            card,
            online: false,
            last_seen_ms: seen,
            public_keys: None,
        }
    }

    /// Answers `query_directory` with whatever `members` holds and
    /// `find_people` with `people` (or an error)
    pub(crate) struct DirectoryRelay {
        pub members: Mutex<Result<Vec<MemberRecord>, String>>,
        pub people: Mutex<Result<PeopleSearch, String>>,
        pub connected: bool,
    }

    impl DirectoryRelay {
        /// Everyone in `cards` online
        pub fn new(cards: Vec<GatewayCard>) -> Self {
            Self::listing(cards.into_iter().map(online).collect())
        }

        pub fn listing(members: Vec<MemberRecord>) -> Self {
            Self {
                members: Mutex::new(Ok(members)),
                people: Mutex::new(Ok(PeopleSearch::default())),
                connected: true,
            }
        }

        pub fn set(&self, members: Vec<MemberRecord>) {
            *self.members.lock().unwrap() = Ok(members);
        }
    }

    impl RelayClient for DirectoryRelay {
        fn connect<'a>(&'a self, _: &'a str, _: &'a str) -> BoxFuture<'a, Result<(), String>> {
            Box::pin(async { Ok(()) })
        }
        fn register(&self, _: Vec<AgentCard>) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async { Ok(()) })
        }
        fn heartbeat(&self, _: HashMap<String, u32>) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async { Ok(()) })
        }
        fn query_capability<'a>(
            &'a self,
            _: &'a str,
        ) -> BoxFuture<'a, Result<Vec<AgentCard>, String>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn delegate_task<'a>(
            &'a self,
            _: &'a Task,
            _: &'a [String],
        ) -> BoxFuture<'a, Result<crate::services::relay::client_api::Submitted, String>> {
            Box::pin(async { Err("unused".to_string()) })
        }
        fn send_result(&self, _: TaskResult) -> BoxFuture<'_, Result<(), String>> {
            Box::pin(async { Ok(()) })
        }
        fn on_task_assigned(&self, _: Handler<TaskAssigned>) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
        fn on_task_completed(&self, _: Handler<TaskCompleted>) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
        fn on_task_failed(&self, _: Handler<TaskFailed>) -> BoxFuture<'_, ()> {
            Box::pin(async {})
        }
        fn status(&self) -> RelayStatus {
            RelayStatus {
                connected: self.connected,
                relay_url: Some("wss://relay.example/relay?token=secret-token".to_string()),
                ..RelayStatus::default()
            }
        }
        fn query_directory(&self) -> BoxFuture<'_, Result<Vec<MemberRecord>, String>> {
            let members = self.members.lock().unwrap().clone();
            Box::pin(async move { members })
        }
        fn find_people<'a>(
            &'a self,
            _: &'a str,
            _: u32,
        ) -> BoxFuture<'a, Result<PeopleSearch, String>> {
            let people = self.people.lock().unwrap().clone();
            Box::pin(async move { people })
        }
    }

    #[tokio::test]
    async fn an_empty_relay_gives_an_empty_directory() {
        let db = Db::in_memory();
        let service = NetworkService::new(&db);
        let relay = DirectoryRelay::new(Vec::new());
        let (snapshot, changes) = service.refresh_with(&db, Some(&relay), "me").await;
        assert!(snapshot.members.is_empty());
        assert!(snapshot.relay_connected);
        // Never with this instance's token (it ends up in invites)
        assert_eq!(
            snapshot.relay_url.as_deref(),
            Some("wss://relay.example/relay")
        );
        assert!(changes.is_empty());

        // No relay at all: still an answer, disconnected
        let (snapshot, _) = service.refresh_with(&db, None, "me").await;
        assert!(!snapshot.relay_connected && snapshot.members.is_empty());
    }

    #[tokio::test]
    async fn one_member_is_built_from_its_card_and_we_are_left_out() {
        let db = Db::in_memory();
        let service = NetworkService::new(&db);
        let relay = DirectoryRelay::new(vec![
            gateway("alice-1", "Alice", "alice"),
            gateway("me", "Me", "me"),
            gateway("anon-1", "", ""),
        ]);
        let (snapshot, changes) = service.refresh_with(&db, Some(&relay), "me").await;
        let ids: Vec<&str> = snapshot
            .members
            .iter()
            .map(|m| m.instance_id.as_str())
            .collect();
        assert_eq!(ids, ["alice-1", "anon-1"]);
        let alice = &snapshot.members[0];
        assert!(alice.online);
        assert_eq!(alice.last_seen, Some(1_000.0));
        assert_eq!(alice.handle.as_deref(), Some("alice"));
        assert_eq!(alice.display_name, "Alice");
        // No name anywhere: the instance ID; no handle
        assert_eq!(snapshot.members[1].display_name, "anon-1");
        assert_eq!(snapshot.members[1].handle, None);
        assert!(changes.membership_changed);
        assert_eq!(changes.came_online.len(), 2);
    }

    #[tokio::test]
    async fn the_relay_lists_offline_members_and_drops_revoked_ones() {
        let db = Db::in_memory();
        let service = NetworkService::new(&db);
        let relay = DirectoryRelay::new(vec![
            gateway("i-e", "eve", "eve"),
            gateway("i-b", "Bob", "bob"),
            gateway("i-d", "dana", "dana"),
            gateway("i-a2", "alice", "alice2"),
            gateway("i-a1", "Alice", "alice"),
        ]);
        let (snapshot, _) = service.refresh_with(&db, Some(&relay), "me").await;
        let ids: Vec<&str> = snapshot
            .members
            .iter()
            .map(|m| m.instance_id.as_str())
            .collect();
        // Duplicate names (case-insensitively) tie-break on instance ID
        assert_eq!(ids, ["i-a1", "i-a2", "i-b", "i-d", "i-e"]);

        // Bob and Dana disconnect: the relay still lists them, offline,
        // with when it last saw them
        relay.set(vec![
            online(gateway("i-e", "eve", "eve")),
            offline(gateway("i-b", "Bob", "bob"), 500.0),
            offline(gateway("i-d", "dana", "dana"), 600.0),
            online(gateway("i-a2", "alice", "alice2")),
            online(gateway("i-a1", "Alice", "alice")),
        ]);
        let (snapshot, changes) = service.refresh_with(&db, Some(&relay), "me").await;
        let order: Vec<(&str, bool)> = snapshot
            .members
            .iter()
            .map(|m| (m.instance_id.as_str(), m.online))
            .collect();
        assert_eq!(
            order,
            [
                ("i-a1", true),
                ("i-a2", true),
                ("i-e", true),
                ("i-b", false),
                ("i-d", false)
            ]
        );
        assert_eq!(snapshot.members[3].last_seen, Some(500.0));
        assert_eq!(changes.went_offline.len(), 2);
        assert!(changes.came_online.is_empty() && !changes.membership_changed);

        // A failing relay: everyone known is offline, nobody is lost
        *relay.members.lock().unwrap() = Err("timeout".to_string());
        let (snapshot, changes) = service.refresh_with(&db, Some(&relay), "me").await;
        assert_eq!(snapshot.members.len(), 5);
        assert!(snapshot.members.iter().all(|m| !m.online));
        assert_eq!(changes.went_offline.len(), 3);

        // Known members survive a restart, offline until the relay answers
        let restarted = NetworkService::new(&db);
        let (snapshot, changes) = restarted.refresh_with(&db, None, "me").await;
        assert_eq!(snapshot.members.len(), 5);
        assert!(changes.is_empty());

        // Back: Bob online; Dana was revoked, so the relay drops her
        relay.set(vec![
            online(gateway("i-b", "Bob", "bob")),
            offline(gateway("i-e", "eve", "eve"), 700.0),
            offline(gateway("i-a2", "alice", "alice2"), 700.0),
            offline(gateway("i-a1", "Alice", "alice"), 700.0),
        ]);
        let (snapshot, changes) = restarted.refresh_with(&db, Some(&relay), "me").await;
        assert_eq!(snapshot.members[0].instance_id, "i-b");
        assert!(snapshot.members[0].online);
        assert_eq!(snapshot.members.len(), 4);
        assert_eq!(changes.came_online.len(), 1);
        assert!(!changes.membership_changed);
    }

    #[tokio::test]
    async fn new_members_join_and_changed_cards_update_profiles() {
        let db = Db::in_memory();
        let service = NetworkService::new(&db);
        let relay = DirectoryRelay::new(vec![gateway("i-a", "Alice", "alice")]);
        let (_, changes) = service.refresh_with(&db, Some(&relay), "me").await;
        assert_eq!(changes.joined.len(), 1);
        assert!(changes.profile_updated.is_empty());

        // Same card: nothing changed
        let (_, changes) = service.refresh_with(&db, Some(&relay), "me").await;
        assert!(changes.is_empty(), "{changes:?}");

        // A new name on the card, and Bob arrives (offline: he joined
        // earlier and left before we looked)
        relay.set(vec![
            online(gateway("i-a", "Alice (Ops)", "alice")),
            offline(gateway("i-b", "Bob", "bob"), 10.0),
        ]);
        let (_, changes) = service.refresh_with(&db, Some(&relay), "me").await;
        let joined: Vec<&str> = changes
            .joined
            .iter()
            .map(|m| m.display_name.as_str())
            .collect();
        assert_eq!(joined, ["Bob"]);
        assert!(changes.came_online.is_empty());
        let updated: Vec<&str> = changes
            .profile_updated
            .iter()
            .map(|m| m.display_name.as_str())
            .collect();
        assert_eq!(updated, ["Alice (Ops)"]);
        assert!(!changes.is_empty());
    }

    #[tokio::test]
    async fn snapshots_are_cached_until_invalidated() {
        let db = Db::in_memory();
        let service = NetworkService::new(&db);
        assert!(service.cached().is_none());
        let relay = DirectoryRelay::new(vec![gateway("i-a", "A", "a")]);
        service.refresh_with(&db, Some(&relay), "me").await;
        assert_eq!(service.cached().unwrap().members.len(), 1);
        service.invalidate();
        assert!(service.cached().is_none());
    }

    #[tokio::test]
    async fn the_service_answers_through_app_state() {
        let state = AppState::for_tests(vec![]);
        assert!(state.network().is_some());
        let snapshot = get_network_snapshot(&state).await.unwrap();
        assert!(snapshot.members.is_empty());
        // No relay: refreshing says so; the listing stays readable
        let refreshed = refresh_network(&state).await.unwrap_err();
        assert!(RelayUnavailable::is(&refreshed), "{refreshed}");
        assert!(get_network_snapshot(&state).await.is_ok());
        state.shutdown();
    }
}
