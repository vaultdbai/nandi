//! Capability and tool presence on the relay. On
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
//!
//! Presence never says more than the cards already do ([`only_shown`]):
//! a provider is published only for a capability the gateway card exposes
//! (`relay.enabled`, `expose`, `hide`) or an agent card serves, a
//! capability whose definition is `private` never is, and the tools sent
//! are only those of the providers published.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use rusqlite::Connection;

use super::client::HEARTBEAT_INTERVAL;
use super::protocol::{CapabilityPresence, InstancePresence, ProviderPresence};
use super::RelayClient;
use crate::db::now_ms;
use crate::services::capability::ProviderKind;
use crate::services::network;
use crate::services::selection::capability::running_tasks;
use crate::services::tools::definition::normalize_id;
use crate::services::{AppState, WeakAppState};

/// This instance's enabled providers in `capability_providers`, grouped by
/// capability (sorted), with their running task counts as load. A
/// capability whose definition is `private` (the default) is left out.
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
           AND (c.definition_json IS NULL
                OR COALESCE(json_extract(c.definition_json, '$.policy.visibility'), 'private')
                   != 'private')
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

/// What this instance already shows others on the relay
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Shown {
    /// The gateway card's capabilities (empty while `relay.enabled` is off)
    pub gateway: BTreeSet<String>,
    /// `(agent ID, capability)` for every agent card (agents that accept
    /// delegation)
    pub cards: BTreeSet<(String, String)>,
}

impl Shown {
    fn shows(&self, capability: &str, provider: &ProviderPresence) -> bool {
        self.gateway.contains(capability)
            || (provider.provider_kind == ProviderKind::Agent
                && self
                    .cards
                    .contains(&(provider.provider_id.clone(), capability.to_string())))
    }
}

/// Only the providers `shown` already shows, and the tools of those
/// providers (sorted): nothing private and no full tool list reaches the
/// relay
pub fn only_shown(
    capabilities: Vec<CapabilityPresence>,
    shown: &Shown,
) -> (Vec<CapabilityPresence>, Vec<String>) {
    let published: Vec<CapabilityPresence> = capabilities
        .into_iter()
        .filter_map(|mut capability| {
            let id = capability.capability_id.clone();
            capability
                .providers
                .retain(|provider| shown.shows(&id, provider));
            (!capability.providers.is_empty()).then_some(capability)
        })
        .collect();
    let tools: BTreeSet<String> = published
        .iter()
        .flat_map(|capability| &capability.providers)
        .flat_map(|provider| provider.provided_tools.iter().cloned())
        .collect();
    (published, tools.into_iter().collect())
}

/// What the cards show: the gateway card's capabilities while sharing is
/// on, and each agent card's served capabilities
async fn shown(state: &AppState, instance_id: &str) -> Shown {
    let mut gateway = BTreeSet::new();
    if let Some(service) = state.gateway() {
        match service.manifest() {
            Ok(manifest) if manifest.relay.enabled => {
                gateway.extend(service.card(&manifest).await.capabilities);
            }
            Ok(_) => {}
            Err(e) => log::warn!("No gateway manifest for presence: {e}"),
        }
    }
    let cards = super::local_cards(state, instance_id, "")
        .into_iter()
        .filter(|card| card.accepts_delegation)
        .flat_map(|card| {
            let agent_id = card.agent_id().to_string();
            card.capabilities
                .into_iter()
                .map(move |capability| (agent_id.clone(), capability.name))
        })
        .collect();
    Shown { gateway, cards }
}

/// This instance's presence: its enabled, not private providers that the
/// cards already show, and their tools. Empty when it can't be read.
pub async fn local_presence(state: &AppState) -> (Vec<CapabilityPresence>, Vec<String>) {
    let instance_id = match network::local_instance_id(state.db()) {
        Ok(instance_id) => instance_id,
        Err(e) => {
            log::warn!("No instance ID for presence: {e}");
            return (Vec::new(), Vec::new());
        }
    };
    let id = instance_id.clone();
    let capabilities = state
        .db()
        .call(move |conn| presence_from_db(conn, &id))
        .await
        .unwrap_or_else(|e| {
            log::warn!("Couldn't read the local capability providers: {e}");
            Vec::new()
        });
    only_shown(capabilities, &shown(state, &instance_id).await)
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
             VALUES ('code-review', '1.2.0', 'Code review',
                     '{\"policy\":{\"visibility\":\"team\"}}'),
                    ('secret-sauce', '1.0.0', 'Private',
                     '{\"policy\":{\"visibility\":\"private\"}}'),
                    ('unspecified', '1.0.0', 'Default', '{}')",
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
                    ('summarize', '0.1.0', 'peer', 'agent', 'theirs', 1, 1, '[]'),
                    ('secret-sauce', NULL, 'me', 'agent', 'reviewer', 1, 1, '[\"aws\"]'),
                    ('unspecified', NULL, 'me', 'agent', 'reviewer', 1, 1, '[]')",
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
        // Private, by its definition or by default, is never published
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

    fn provider(kind: ProviderKind, id: &str, tools: &[&str]) -> ProviderPresence {
        ProviderPresence {
            provider_kind: kind,
            provider_id: id.into(),
            load: 0,
            max_concurrent: 1,
            provided_tools: tools.iter().map(|t| t.to_string()).collect(),
            online_since: 0.0,
            last_heartbeat: 0.0,
        }
    }

    fn capability(id: &str, providers: Vec<ProviderPresence>) -> CapabilityPresence {
        CapabilityPresence {
            capability_id: id.into(),
            capability_version: String::new(),
            providers,
        }
    }

    fn local() -> Vec<CapabilityPresence> {
        vec![
            capability(
                "code-review",
                vec![
                    provider(ProviderKind::Agent, "reviewer", &["builtin.shell"]),
                    provider(ProviderKind::Workflow, "review-flow", &[]),
                ],
            ),
            capability(
                "payroll",
                vec![provider(ProviderKind::Agent, "hr-bot", &["builtin.aws"])],
            ),
        ]
    }

    #[test]
    fn nothing_is_published_that_no_card_shows() {
        let (capabilities, tools) = only_shown(local(), &Shown::default());
        assert!(capabilities.is_empty());
        assert!(tools.is_empty());
    }

    #[test]
    fn an_agent_card_publishes_only_its_own_capabilities_and_tools() {
        let shown = Shown {
            gateway: BTreeSet::new(),
            cards: [("reviewer".to_string(), "code-review".to_string())].into(),
        };
        let (capabilities, tools) = only_shown(local(), &shown);
        assert_eq!(capabilities.len(), 1);
        let ids: Vec<&str> = capabilities[0]
            .providers
            .iter()
            .map(|p| p.provider_id.as_str())
            .collect();
        // The workflow isn't on a card; payroll's agent doesn't delegate
        assert_eq!(ids, ["reviewer"]);
        assert_eq!(tools, ["builtin.shell"]);
    }

    #[test]
    fn the_gateway_publishes_every_provider_of_what_it_exposes() {
        let shown = Shown {
            gateway: ["code-review".to_string()].into(),
            cards: BTreeSet::new(),
        };
        let (capabilities, tools) = only_shown(local(), &shown);
        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0].providers.len(), 2);
        // Not exposed: payroll's tools stay home
        assert_eq!(tools, ["builtin.shell"]);
    }
}
