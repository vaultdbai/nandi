//! Capability and tool presence on the relay (wave A, Track D). On
//! register (and with each heartbeat) an instance tells the relay which
//! capabilities its agents and workflows provide and which tools it has;
//! requesters ask with `CapabilityPresenceQuery`.
//!
//! - [`publish`] sets the presence on the relay client before it
//!   registers, so `RegisterInstance` carries it (and every reconnect).
//! - [`spawn_refresh`] recomputes it every heartbeat interval and sends
//!   `UpdatePresence` when it changed (a provider registered, a task
//!   started or finished, a tool was enabled). The relay stamps each
//!   provider with the instance's last heartbeat.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rusqlite::Connection;

use super::client::HEARTBEAT_INTERVAL;
use super::protocol::{CapabilityPresence, InstancePresence, ProviderPresence};
use super::RelayClient;
use crate::db::now_ms;
use crate::services::network;
use crate::services::selection::capability::running_tasks;
use crate::services::tools::definition::normalize_id;
use crate::services::{AppState, WeakAppState};

/// This instance's enabled providers in `capability_providers`, grouped by
/// capability (sorted), with their running task counts as load
pub fn presence_from_db(
    conn: &Connection,
    instance_id: &str,
) -> rusqlite::Result<Vec<CapabilityPresence>> {
    let mut stmt = conn.prepare(
        "SELECT p.capability_id,
                COALESCE(p.capability_version, c.version, ''),
                p.provider_kind, p.provider_id, p.max_concurrent, p.provided_tools
         FROM capability_providers p
         LEFT JOIN capabilities_projection c ON c.id = p.capability_id
         WHERE p.instance_id = ?1 AND COALESCE(p.enabled, 1) != 0
         ORDER BY p.capability_id, p.provider_id",
    )?;
    let rows = stmt
        .query_map([instance_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let now = now_ms() as f64;
    let mut grouped: BTreeMap<String, CapabilityPresence> = BTreeMap::new();
    for (capability_id, version, kind, provider_id, max, tools) in rows {
        let mut provided_tools: Vec<String> = tools
            .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
            .unwrap_or_default()
            .iter()
            .map(|t| normalize_id(t))
            .collect();
        provided_tools.sort();
        provided_tools.dedup();
        let provider = ProviderPresence {
            provider_kind: serde_json::from_value(serde_json::Value::String(kind))
                .unwrap_or_default(),
            load: running_tasks(conn, instance_id, &provider_id)?,
            provider_id,
            max_concurrent: max.map_or(1, |m| m.max(1) as u32),
            provided_tools,
            online_since: now,
            last_heartbeat: now,
        };
        grouped
            .entry(capability_id.clone())
            .or_insert_with(|| CapabilityPresence {
                capability_id,
                capability_version: version,
                providers: Vec::new(),
            })
            .providers
            .push(provider);
    }
    Ok(grouped.into_values().collect())
}

/// This instance's presence, from `capability_providers` and the tool
/// catalog (its enabled tools). Empty when either can't be read.
pub async fn local_presence(state: &AppState) -> (Vec<CapabilityPresence>, Vec<String>) {
    let capabilities = match network::local_instance_id(state.db()) {
        Ok(instance_id) => state
            .db()
            .call(move |conn| presence_from_db(conn, &instance_id))
            .await
            .unwrap_or_else(|e| {
                log::warn!("Couldn't read the local capability providers: {e}");
                Vec::new()
            }),
        Err(e) => {
            log::warn!("No instance ID for presence: {e}");
            Vec::new()
        }
    };
    let mut tools: Vec<String> = match state.tools() {
        Some(service) => service
            .list_all()
            .await
            .map(|entries| {
                entries
                    .into_iter()
                    .filter(|entry| entry.enabled)
                    .map(|entry| entry.definition.id)
                    .collect()
            })
            .unwrap_or_default(),
        None => Vec::new(),
    };
    tools.sort();
    tools.dedup();
    (capabilities, tools)
}

/// The relay's answer for `capability_id` (`*`: everything)
pub async fn query_capability_presence(
    state: &AppState,
    capability_id: &str,
) -> Result<Vec<InstancePresence>, String> {
    let capability_id = capability_id.trim();
    if capability_id.is_empty() {
        return Err("Choose a capability".to_string());
    }
    let client = super::unavailable::connected(state.relay_client())?;
    client.query_capability_presence(capability_id).await
}

/// Sets the current presence on `client` (sent with the next registration,
/// or now when registered). Returns what it set.
pub async fn publish(
    state: &AppState,
    client: &dyn RelayClient,
) -> (Vec<CapabilityPresence>, Vec<String>) {
    let (capabilities, tools) = local_presence(state).await;
    if let Err(e) = client
        .set_presence(capabilities.clone(), tools.clone())
        .await
    {
        log::debug!("Relay presence not sent: {e}");
    }
    (capabilities, tools)
}

/// One refresh loop per process
static REFRESHING: AtomicBool = AtomicBool::new(false);

/// Every heartbeat interval: recompute the presence and send it when it
/// changed. Stops when the app goes away.
pub fn spawn_refresh(state: &AppState, last: (Vec<CapabilityPresence>, Vec<String>)) {
    if REFRESHING.swap(true, Ordering::SeqCst) {
        return;
    }
    let weak: WeakAppState = state.downgrade();
    state.runtime().spawn(async move {
        let mut last = strip_times(last);
        loop {
            tokio::time::sleep(HEARTBEAT_INTERVAL).await;
            let Some(state) = weak.upgrade() else {
                break;
            };
            let Some(client) = state.relay_client().cloned() else {
                continue;
            };
            let next = local_presence(&state).await;
            let comparable = strip_times(next.clone());
            if comparable == last {
                continue;
            }
            match set(&client, next).await {
                Ok(()) => last = comparable,
                Err(e) => log::debug!("Relay presence refresh failed: {e}"),
            }
        }
        REFRESHING.store(false, Ordering::SeqCst);
    });
}

async fn set(
    client: &Arc<dyn RelayClient>,
    (capabilities, tools): (Vec<CapabilityPresence>, Vec<String>),
) -> Result<(), String> {
    client.set_presence(capabilities, tools).await
}

/// Presence without the timestamps, to tell a real change from a tick
fn strip_times(
    (mut capabilities, tools): (Vec<CapabilityPresence>, Vec<String>),
) -> (Vec<CapabilityPresence>, Vec<String>) {
    for capability in &mut capabilities {
        for provider in &mut capability.providers {
            provider.online_since = 0.0;
            provider.last_heartbeat = 0.0;
        }
    }
    (capabilities, tools)
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;

    use super::*;
    use crate::db::Db;
    use crate::services::capability::ProviderKind;
    use crate::services::relay::client::WsRelayClient;
    use nandi_relay::{issue_instance_token, serve};

    const SECRET: &[u8] = b"presence-secret";

    async fn connected(addr: SocketAddr, instance_id: &str) -> WsRelayClient {
        let token = issue_instance_token(SECRET, instance_id);
        let client = WsRelayClient::new(None, instance_id);
        client
            .connect(&format!("ws://{addr}/relay?token={token}"), instance_id)
            .await
            .unwrap();
        client
    }

    fn reviewing(load: u32) -> Vec<CapabilityPresence> {
        vec![CapabilityPresence {
            capability_id: "code-review".into(),
            capability_version: "1.0.0".into(),
            providers: vec![ProviderPresence {
                provider_kind: ProviderKind::Agent,
                provider_id: "reviewer".into(),
                load,
                max_concurrent: 2,
                provided_tools: vec!["builtin.shell".into()],
                online_since: 0.0,
                last_heartbeat: 0.0,
            }],
        }]
    }

    #[tokio::test]
    async fn registration_carries_presence_and_updates_replace_it() {
        let addr = serve(SocketAddr::from(([127, 0, 0, 1], 0)), SECRET.to_vec())
            .await
            .unwrap();
        let provider = connected(addr, "prov").await;
        // Set before registering: `RegisterInstance` carries it
        provider
            .set_presence(reviewing(0), vec!["builtin.shell".into()])
            .await
            .unwrap();
        provider.register(vec![]).await.unwrap();
        // A peer that provides nothing
        let old = connected(addr, "old").await;
        old.register(vec![]).await.unwrap();
        let requester = connected(addr, "req").await;
        requester.register(vec![]).await.unwrap();

        let found = requester
            .query_capability_presence("code-review")
            .await
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].instance_id, "prov");
        assert_eq!(found[0].capability_version, "1.0.0");
        assert_eq!(found[0].provider.provider_id, "reviewer");
        assert_eq!(found[0].provider.provided_tools, ["builtin.shell"]);
        // The relay stamps its heartbeat
        assert!(found[0].provider.last_heartbeat > 0.0);
        assert!(requester
            .query_capability_presence("translate")
            .await
            .unwrap()
            .is_empty());

        // Registered now: `UpdatePresence` replaces it
        provider
            .set_presence(reviewing(2), vec!["builtin.shell".into()])
            .await
            .unwrap();
        let mut load = 0;
        for _ in 0..400 {
            let all = requester.query_capability_presence("*").await.unwrap();
            load = all.first().map_or(0, |p| p.provider.load);
            if load == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(load, 2);
    }

    #[test]
    fn groups_local_providers_by_capability() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        conn.execute(
            "INSERT INTO capabilities_projection (id, version, name, definition_json)
             VALUES ('code-review', '1.2.0', 'Code review', '{}')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO capability_providers
                 (capability_id, capability_version, instance_id, provider_kind, provider_id,
                  enabled, max_concurrent, provided_tools)
             VALUES ('code-review', NULL, 'me', 'agent', 'reviewer', 1, 2, '[\"shell\"]'),
                    ('code-review', NULL, 'me', 'workflow', 'review-flow', 1, NULL, NULL),
                    ('summarize', '0.1.0', 'me', 'agent', 'reviewer', 1, 1, '[]'),
                    ('summarize', '0.1.0', 'me', 'agent', 'off', 0, 1, '[]'),
                    ('summarize', '0.1.0', 'peer', 'agent', 'theirs', 1, 1, '[]')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, requester_instance, requester_agent, provider_instance,
                 provider_agent, capability, state, created_at, updated_at)
             VALUES ('t1', 'peer', 'peer/a', 'me', 'reviewer', 'code-review', 'working', 0, 0),
                    ('t2', 'peer', 'peer/a', 'me', 'reviewer', 'code-review', 'completed', 0, 0)",
            [],
        )
        .unwrap();
        let presence = presence_from_db(&conn, "me").unwrap();
        assert_eq!(presence.len(), 2);
        let review = &presence[0];
        assert_eq!(review.capability_id, "code-review");
        assert_eq!(review.capability_version, "1.2.0");
        let ids: Vec<&str> = review
            .providers
            .iter()
            .map(|p| p.provider_id.as_str())
            .collect();
        assert_eq!(ids, ["review-flow", "reviewer"]);
        let flow = &review.providers[0];
        assert_eq!(flow.provider_kind, ProviderKind::Workflow);
        assert_eq!(flow.max_concurrent, 1);
        let reviewer = &review.providers[1];
        assert_eq!(reviewer.load, 1);
        assert_eq!(reviewer.provided_tools, ["builtin.shell"]);
        let summarize = &presence[1];
        assert_eq!(summarize.capability_version, "0.1.0");
        assert_eq!(summarize.providers.len(), 1);
    }
}
