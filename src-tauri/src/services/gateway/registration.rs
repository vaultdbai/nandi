//! The gateway card and its relay registration (wave final, Track D):
//! capabilities from local agents filtered by `expose` / `hide`, load from
//! local tasks in `Working`; registered when `relay.enabled`, heartbeat
//! every 30s; with `federation.enabled`, trusted peers' cards are followed.
//!
//! Everything here is pure except [`current_load`]; the loop that calls it
//! lives in `GatewayService`.

use std::collections::BTreeSet;
use std::time::Duration;

use rusqlite::{params, Connection};

use super::policy::is_exposed;
use super::{GatewayCard, GatewayManifest, GatewayPolicy, GATEWAY_AGENT_ID, GATEWAY_VERSION};
use crate::services::a2a::task::TaskState;
use crate::services::domain::agent_card::OwnerRef;
use crate::services::domain::capability::ServesCapability;
use crate::services::domain::AgentCard;
use crate::services::network;

/// How often the card is re-sent
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);
/// How often NANDI.md's modification time is checked
pub const POLL_INTERVAL: Duration = Duration::from_secs(5);
/// How often trusted peers' cards are refreshed (federation on)
pub const FEDERATION_INTERVAL: Duration = Duration::from_secs(60);

/// The capabilities the gateway offers: the union of what local agents
/// serve, within `expose` (when it lists any), minus `hide`. Sorted,
/// distinct.
pub fn exposed_capabilities<'a>(
    local: impl IntoIterator<Item = &'a str>,
    manifest: &GatewayManifest,
) -> Vec<String> {
    local
        .into_iter()
        .filter(|capability| is_exposed(&manifest.policy, capability))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// `cards` with only the capabilities within `expose` / `hide`; a card
/// left serving nothing is dropped
pub fn exposed_cards(cards: Vec<AgentCard>, policy: &GatewayPolicy) -> Vec<AgentCard> {
    cards
        .into_iter()
        .filter_map(|mut card| {
            card.capabilities.retain(|c| is_exposed(policy, &c.name));
            (!card.capabilities.is_empty()).then_some(card)
        })
        .collect()
}

/// The agent cards the relay may list: none while sharing is off
/// (`relay.enabled`), otherwise the exposed ones. Presence follows them,
/// so a hidden agent is neither listed nor offered.
pub fn shared_agent_cards(cards: Vec<AgentCard>, manifest: &GatewayManifest) -> Vec<AgentCard> {
    if !manifest.relay.enabled {
        return Vec::new();
    }
    exposed_cards(cards, &manifest.policy)
}

/// The card that tells the relay sharing is off: who this is and nothing
/// more. The relay takes the member out of the directory on it; an older
/// relay keeps it, so it carries no profile.
pub fn withdrawn_card(card: GatewayCard) -> GatewayCard {
    GatewayCard {
        accepts_delegation: false,
        description: String::new(),
        capabilities: Vec::new(),
        current_load: 0,
        agent_count: 0,
        body: None,
        responsibilities: Vec::new(),
        direct_delegation: Some(false),
        availability: Default::default(),
        ..card
    }
}

/// Who the card names: the manifest's owner, the user slug standing in
/// for a missing handle (the handle field carries the slug now)
pub fn card_owner(manifest: &GatewayManifest) -> OwnerRef {
    let mut owner = manifest.owner.clone();
    if owner.github_handle.trim().is_empty() {
        owner.github_handle = manifest.user_slug.clone();
    }
    if owner.display_name.trim().is_empty() {
        owner.display_name = manifest.display_name.clone();
    }
    owner
}

/// This instance's gateway card. `relay_url` may be empty (no relay yet:
/// no endpoint).
pub fn build_card(
    manifest: &GatewayManifest,
    instance_id: &str,
    relay_url: &str,
    local_cards: &[AgentCard],
    current_load: u32,
) -> GatewayCard {
    let capabilities = exposed_capabilities(
        local_cards
            .iter()
            .flat_map(|card| card.capabilities.iter().map(|c| c.name.as_str())),
        manifest,
    );
    let agent_count = local_cards
        .iter()
        .filter(|card| {
            card.capabilities
                .iter()
                .any(|c| capabilities.contains(&c.name))
        })
        .count() as u32;
    let endpoint = if relay_url.trim().is_empty() {
        String::new()
    } else {
        network::agent_endpoint(relay_url, instance_id, GATEWAY_AGENT_ID)
    };
    let display_name = if manifest.display_name.trim().is_empty() {
        manifest.name.clone()
    } else {
        manifest.display_name.clone()
    };
    GatewayCard {
        instance_id: instance_id.to_string(),
        endpoint,
        display_name,
        description: manifest.description.clone(),
        accepts_delegation: manifest.relay.enabled,
        capabilities,
        max_concurrent: manifest.policy.max_concurrent,
        current_load,
        owner: card_owner(manifest),
        federation_enabled: manifest.federation.enabled,
        gateway_version: GATEWAY_VERSION,
        agent_count,
        body: manifest
            .relay
            .discoverable
            .then(|| super::manifest::public_body(&manifest.body)),
        // Not gated by `discoverable` (see `GatewayCard::responsibilities`),
        // only by its own switch
        responsibilities: if manifest.relay.share_responsibilities {
            super::manifest::normalize_responsibilities(manifest.responsibilities.clone())
        } else {
            Vec::new()
        },
        direct_delegation: Some(local_cards.iter().any(|card| card.accepts_delegation)),
        availability: manifest.availability.clone(),
    }
}

/// The agent card the relay routes gateway tasks by: `<instance>/gateway`,
/// serving [`GATEWAY_AGENT_ID`]. Relays route `DelegateTask` to agent
/// cards only, so without it a task for a gateway is refused ("No online
/// agent … serves gateway"). It also makes a gateway-only instance count
/// as online in presence.
pub fn route_card(card: &GatewayCard) -> AgentCard {
    AgentCard {
        id: AgentCard::card_id(&card.instance_id, GATEWAY_AGENT_ID),
        name: card.display_name.clone(),
        description: card.description.clone(),
        instance_id: card.instance_id.clone(),
        endpoint: card.endpoint.clone(),
        capabilities: vec![ServesCapability {
            name: GATEWAY_AGENT_ID.to_string(),
            description: "Natural-language requests to this Nandi's gateway".to_string(),
            input_schema: None,
            output_schema: None,
        }],
        accepts_delegation: card.accepts_delegation,
        max_concurrent: card.max_concurrent,
        current_load: card.current_load,
        owner: card.owner.clone(),
        signature: None,
        issued_at: 0.0,
    }
}

/// Whether `card` is a gateway's [`route_card`], not an agent
pub fn is_route_card(card: &AgentCard) -> bool {
    card.agent_id() == GATEWAY_AGENT_ID
}

/// Tasks this instance is working on as the provider
pub fn current_load(conn: &Connection, instance_id: &str) -> rusqlite::Result<u32> {
    conn.query_row(
        "SELECT COUNT(*) FROM tasks WHERE state = ?1 AND provider_instance = ?2",
        params![TaskState::Working.as_str(), instance_id],
        |row| row.get(0),
    )
}

/// Whether `owner` is one of `trusted`: same handle, else same display
/// name (case-insensitive, empty never matches)
pub fn is_trusted_owner(owner: &OwnerRef, trusted: &[OwnerRef]) -> bool {
    let same = |a: &str, b: &str| !a.trim().is_empty() && a.trim().eq_ignore_ascii_case(b.trim());
    trusted.iter().any(|peer| {
        same(&peer.github_handle, &owner.github_handle)
            || (peer.github_handle.trim().is_empty()
                && same(&peer.display_name, &owner.display_name))
    })
}

/// The cards of trusted peers among `cards`, never our own
pub fn trusted_cards(
    cards: Vec<GatewayCard>,
    trusted: &[OwnerRef],
    local_instance: &str,
) -> Vec<GatewayCard> {
    cards
        .into_iter()
        .filter(|card| card.instance_id != local_instance && is_trusted_owner(&card.owner, trusted))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn agent(id: &str, capabilities: &[&str]) -> AgentCard {
        AgentCard {
            id: format!("me/{id}"),
            name: id.to_string(),
            description: String::new(),
            instance_id: "me".into(),
            endpoint: String::new(),
            capabilities: capabilities
                .iter()
                .map(|name| ServesCapability {
                    name: name.to_string(),
                    description: String::new(),
                    input_schema: None,
                    output_schema: None,
                })
                .collect(),
            accepts_delegation: true,
            max_concurrent: 1,
            current_load: 0,
            owner: OwnerRef::default(),
            signature: None,
            issued_at: 0.0,
        }
    }

    fn manifest() -> GatewayManifest {
        let mut manifest =
            super::super::manifest::parse(&super::super::manifest::render_default("Ada")).unwrap();
        manifest.user_slug = "ada".into();
        manifest
    }

    #[test]
    fn the_card_offers_the_union_of_local_capabilities() {
        let agents = [
            agent("papers", &["find-papers", "summarize"]),
            agent("writer", &["summarize", "write"]),
            agent("coder", &["code"]),
        ];
        let mut manifest = manifest();
        let card = build_card(&manifest, "me", "wss://relay.example/relay", &agents, 2);
        assert_eq!(
            card.capabilities,
            ["code", "find-papers", "summarize", "write"]
        );
        assert_eq!(card.agent_count, 3);
        assert_eq!(card.endpoint, "wss://relay.example/a/me/gateway");
        assert_eq!(card.current_load, 2);
        assert_eq!(card.owner.github_handle, "ada");
        assert_eq!(card.display_name, "Ada");
        assert!(card.body.is_none());
        assert_eq!(
            card.direct_delegation,
            Some(true),
            "the test agents accept it"
        );
        let closed: Vec<AgentCard> = agents
            .iter()
            .cloned()
            .map(|mut card| {
                card.accepts_delegation = false;
                card
            })
            .collect();
        let gateway_only = build_card(&manifest, "me", "", &closed, 0);
        assert_eq!(gateway_only.direct_delegation, Some(false));

        // `hide` removes; `expose` (when set) limits
        manifest.policy.hide = vec!["write".into()];
        let card = build_card(&manifest, "me", "", &agents, 0);
        assert_eq!(card.capabilities, ["code", "find-papers", "summarize"]);
        assert_eq!(card.endpoint, "");
        manifest.policy.expose = vec!["summarize".into(), "write".into(), "missing".into()];
        let card = build_card(&manifest, "me", "", &agents, 0);
        assert_eq!(card.capabilities, ["summarize"]);
        assert_eq!(card.agent_count, 2);

        // The body only when discoverable
        manifest.relay.discoverable = true;
        let card = build_card(&manifest, "me", "", &agents, 0);
        assert_eq!(
            card.body.as_deref(),
            Some(super::super::manifest::public_body(&manifest.body).as_str())
        );
        assert!(!card.body.unwrap().contains("<!--"));
    }

    #[test]
    fn agents_are_shared_only_while_sharing_is_on_and_as_exposed() {
        let agents = vec![
            agent("papers", &["find-papers", "summarize"]),
            agent("writer", &["write"]),
        ];
        let mut manifest = manifest();
        assert!(
            shared_agent_cards(agents.clone(), &manifest).is_empty(),
            "sharing is off by default"
        );
        manifest.relay.enabled = true;
        assert_eq!(shared_agent_cards(agents.clone(), &manifest).len(), 2);
        // A hidden capability leaves the card; an agent left with nothing goes
        manifest.policy.hide = vec!["write".into(), "SUMMARIZE".into()];
        let shared = shared_agent_cards(agents.clone(), &manifest);
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].capabilities.len(), 1);
        assert_eq!(shared[0].capabilities[0].name, "find-papers");
        manifest.policy.hide.clear();
        manifest.policy.expose = vec!["write".into()];
        let shared = shared_agent_cards(agents, &manifest);
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].capabilities[0].name, "write");
    }

    #[test]
    fn the_withdrawn_card_says_who_and_nothing_more() {
        let mut manifest = manifest();
        manifest.relay.enabled = true;
        manifest.relay.discoverable = true;
        manifest.responsibilities = vec!["Risk".into()];
        manifest.availability.status = nandi_protocol::AvailabilityStatus::Away;
        let card = build_card(&manifest, "me", "", &[agent("papers", &["summarize"])], 1);
        let withdrawn = withdrawn_card(card.clone());
        assert!(!withdrawn.accepts_delegation);
        assert_eq!(withdrawn.instance_id, "me");
        assert_eq!(withdrawn.display_name, card.display_name);
        assert_eq!(withdrawn.owner, card.owner);
        assert!(withdrawn.description.is_empty());
        assert!(withdrawn.capabilities.is_empty());
        assert!(withdrawn.body.is_none());
        assert!(withdrawn.responsibilities.is_empty());
        assert_eq!(withdrawn.agent_count, 0);
        assert_eq!(withdrawn.availability, Default::default());
    }

    #[test]
    fn responsibilities_stay_off_the_card_when_not_shared() {
        let mut manifest = manifest();
        manifest.relay.enabled = true;
        manifest.responsibilities = vec!["Risk".into()];
        assert!(manifest.relay.share_responsibilities, "shared by default");
        manifest.relay.share_responsibilities = false;
        let card = build_card(&manifest, "me", "", &[], 0);
        assert!(card.responsibilities.is_empty());
    }

    #[test]
    fn responsibilities_are_on_the_card_even_when_not_discoverable() {
        let mut manifest = manifest();
        manifest.relay.enabled = true;
        manifest.relay.discoverable = false;
        manifest.responsibilities = vec!["SOX audit".into(), " sox AUDIT ".into(), "Risk".into()];
        let card = build_card(&manifest, "me", "", &[], 0);
        assert_eq!(card.responsibilities, ["SOX audit", "Risk"]);
        assert!(card.body.is_none(), "the body stays private");
        manifest.relay.discoverable = true;
        let card = build_card(&manifest, "me", "", &[], 0);
        assert_eq!(card.responsibilities, ["SOX audit", "Risk"]);
        assert!(card.body.is_some());
        // Cards from older Nandis have none
        let mut json = serde_json::to_value(&card).unwrap();
        json.as_object_mut().unwrap().remove("responsibilities");
        let old: GatewayCard = serde_json::from_value(json).unwrap();
        assert!(old.responsibilities.is_empty());
    }

    #[test]
    fn counts_working_tasks_as_load() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        for (id, provider, state) in [
            ("t1", "me", "working"),
            ("t2", "me", "working"),
            ("t3", "me", "completed"),
            ("t4", "other", "working"),
        ] {
            conn.execute(
                "INSERT INTO tasks (id, requester_instance, requester_agent, provider_instance,
                     provider_agent, capability, state, created_at, updated_at)
                 VALUES (?1, 'x', 'x/a', ?2, 'gateway', 'c', ?3, 0, 0)",
                params![id, provider, state],
            )
            .unwrap();
        }
        assert_eq!(current_load(&conn, "me").unwrap(), 2);
    }

    #[test]
    fn follows_only_trusted_peers() {
        let owner = |handle: &str, name: &str| OwnerRef {
            github_handle: handle.into(),
            display_name: name.into(),
        };
        let trusted = [owner("grace", ""), owner("", "Alan Turing")];
        let card = |instance: &str, handle: &str, name: &str| GatewayCard {
            owner: owner(handle, name),
            instance_id: instance.into(),
            ..build_card(&manifest(), instance, "", &[], 0)
        };
        let cards = vec![
            card("g", "Grace", "Grace H"),
            card("a", "alan", "alan turing"),
            card("m", "mallory", "Grace"),
            card("me", "grace", ""),
        ];
        let kept: Vec<String> = trusted_cards(cards, &trusted, "me")
            .into_iter()
            .map(|c| c.instance_id)
            .collect();
        assert_eq!(kept, ["g", "a"]);
    }
}
