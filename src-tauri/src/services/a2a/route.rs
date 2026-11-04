//! The network half of picking a provider: asking the relay.
//!
//! The app decides local or network; the relay decides who on the
//! network. This module ranks nothing:
//!
//! 1. A named card on this instance runs here.
//! 2. One of this Nandi's own agents or workflows serves the capability:
//!    it runs here, with no relay (`selection::capability` orders them).
//! 3. Otherwise the relay plans it (`RouteQuery` → `RoutePlan`): who, in
//!    order, and why. My pins from settings.yaml go as `preferred`, the
//!    members I paused or blocked (and anyone this request already failed
//!    with) as `excluded`; the relay never stores them. A named person goes
//!    as the target and is never swapped, except for their own
//!    away-delegate.
//!
//! Without a relay there is no plan: a network request answers
//! `RelayUnavailable`. Local providers never wait for it.

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::json;

use super::assignment::{ASK_ME, ASSIGN_ME};
use super::delegator::instance_of;
use crate::db::Db;
use crate::services::relay::protocol::{
    NoRoute, RequestKind, RouteCandidate, RoutePlan, RouteReason, RouteTarget, RoutingHints,
    RoutingQuery,
};
use crate::services::relay::unavailable;
use crate::services::relay::RelayClient;
use crate::services::repo_store::RepoStore;
use crate::services::team::trust::{TrustFilter, ALL_AGENTS};

/// The task message that says why its provider was picked:
/// `{"provider", "reasons", "redirectedFrom"}` (the relay's reasons,
/// typed, for the thread to render)
pub const ROUTE_KIND: &str = "route";

/// One provider to try
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    /// The card to send to (`<instance>/<agent>`)
    pub target: String,
    /// The owner's GitHub handle, when known
    pub owner: Option<String>,
    /// Who they are, for the thread (the relay's display name)
    pub name: Option<String>,
    /// Why the relay picked them; empty for local providers
    pub reasons: Vec<RouteReason>,
    /// The person they cover for
    pub redirected_from: Option<String>,
}

impl Pick {
    /// A provider picked without the relay: local, or named and sent as is
    pub fn direct(target: impl Into<String>, owner: Option<String>) -> Self {
        Self {
            target: target.into(),
            owner,
            name: None,
            reasons: Vec::new(),
            redirected_from: None,
        }
    }

    pub fn instance(&self) -> &str {
        instance_of(&self.target)
    }

    /// The relay has them free (online with room)
    pub fn is_free(&self) -> bool {
        self.reasons
            .iter()
            .any(|reason| matches!(reason, RouteReason::Free { .. }))
    }

    /// The `route` message for this pick, when the relay gave reasons
    pub fn route_message(&self) -> Option<serde_json::Value> {
        (!self.reasons.is_empty()).then(|| {
            json!({
                "provider": self.name,
                "reasons": self.reasons,
                "redirectedFrom": self.redirected_from,
            })
        })
    }
}

impl From<RouteCandidate> for Pick {
    fn from(candidate: RouteCandidate) -> Self {
        Self {
            target: candidate.target,
            owner: candidate.handle,
            name: Some(candidate.name),
            reasons: candidate.reasons,
            redirected_from: candidate.redirected_from,
        }
    }
}

/// What the relay is asked to route for `capability`
pub fn kind_of(capability: &str) -> RequestKind {
    match capability {
        ASK_ME => RequestKind::Ask,
        ASSIGN_ME => RequestKind::Assign,
        _ => RequestKind::Delegate,
    }
}

/// The members I paused or blocked, as `excluded` hints: an instance ID
/// for a paused member or a whole blocked one, else the blocked card
pub fn excluded_by_me(trust: &TrustFilter) -> Vec<String> {
    let mut excluded: Vec<String> = trust.paused.iter().cloned().collect();
    for (instance, agent) in &trust.blocked {
        excluded.push(if agent == ALL_AGENTS {
            instance.clone()
        } else {
            format!("{instance}/{agent}")
        });
    }
    excluded.sort();
    excluded.dedup();
    excluded
}

/// The hints for a `capability` request: my pin for it, and everyone I
/// paused, blocked or already `tried` (instance IDs). A trust table that
/// can't be read excludes nobody (logged).
pub fn hints(
    store: &RepoStore,
    db: &Db,
    capability: &str,
    tried: &HashSet<String>,
) -> RoutingHints {
    let pin = store.read(|repo| {
        repo.settings()
            .preferred_provider(capability)
            .map(str::to_string)
    });
    let mut excluded = match TrustFilter::load(db) {
        Ok(trust) => excluded_by_me(&trust),
        Err(e) => {
            log::warn!("Routing without my pauses and blocks: {e}");
            Vec::new()
        }
    };
    excluded.extend(tried.iter().cloned());
    excluded.sort();
    excluded.dedup();
    RoutingHints {
        preferred: pin.into_iter().collect(),
        excluded,
        ..RoutingHints::default()
    }
}

/// The query for `capability`, to `target` (a card ID) when one is named
pub fn query(capability: &str, target: Option<&str>, hints: RoutingHints) -> RoutingQuery {
    RoutingQuery {
        kind: kind_of(capability),
        capability: capability.to_string(),
        target: match target {
            Some(card_id) => RouteTarget::Agent {
                card_id: card_id.to_string(),
            },
            None => RouteTarget::Anyone,
        },
        hints,
    }
}

/// The relay's plan for `query`; `RelayUnavailable` without a relay
pub async fn plan(
    relay: Option<&Arc<dyn RelayClient>>,
    query: &RoutingQuery,
) -> Result<RoutePlan, String> {
    let relay = unavailable::configured(relay)?;
    relay.route(query).await
}

/// The plan's candidates as picks, best first
pub fn picks(plan: RoutePlan) -> Vec<Pick> {
    plan.candidates.into_iter().map(Pick::from).collect()
}

/// Why nobody on the network can take a `capability` request
pub fn no_route_message(capability: &str, no_route: Option<NoRoute>) -> String {
    match no_route {
        Some(NoRoute::NotTakingRequests) => format!(
            "The agents that offer '{capability}' don't take requests from other Nandis yet. \
             Their owner can turn that on in the agent's settings."
        ),
        Some(NoRoute::NobodyAvailable) => {
            format!("Nobody who serves '{capability}' can take it right now")
        }
        Some(NoRoute::NobodyServes) | None => {
            format!("No agent that serves '{capability}' is online and accepts delegated tasks")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::relay::protocol::RouteReason;

    #[test]
    fn people_capabilities_are_asks_and_assignments() {
        assert_eq!(kind_of(ASK_ME), RequestKind::Ask);
        assert_eq!(kind_of(ASSIGN_ME), RequestKind::Assign);
        assert_eq!(kind_of("summarize"), RequestKind::Delegate);
    }

    #[test]
    fn paused_and_blocked_members_go_as_exclusions() {
        let trust = TrustFilter {
            paused: HashSet::from(["paused".to_string()]),
            blocked: HashSet::from([
                ("bob".to_string(), "papers".to_string()),
                ("eve".to_string(), ALL_AGENTS.to_string()),
            ]),
        };
        assert_eq!(excluded_by_me(&trust), ["bob/papers", "eve", "paused"]);
    }

    #[test]
    fn a_named_card_is_the_target() {
        let named = query("summarize", Some("bob/papers"), RoutingHints::default());
        assert_eq!(
            named.target,
            RouteTarget::Agent {
                card_id: "bob/papers".into()
            }
        );
        assert_eq!(
            query(ASK_ME, None, RoutingHints::default()).kind,
            RequestKind::Ask
        );
    }

    #[test]
    fn a_plan_becomes_picks_with_their_reasons() {
        let candidate = RouteCandidate {
            target: "bob/papers".into(),
            instance_id: "bob".into(),
            name: "Bob".into(),
            handle: Some("bob".into()),
            agent_name: "Papers".into(),
            provider_kind: Default::default(),
            online: true,
            load: 0,
            max_concurrent: 2,
            wait_seconds: None,
            redirected_from: Some("alice".into()),
            reasons: vec![RouteReason::Free { name: "Bob".into() }],
            public_keys: None,
        };
        let picks = picks(RoutePlan {
            candidates: vec![candidate],
            ..RoutePlan::default()
        });
        assert_eq!(picks[0].target, "bob/papers");
        assert_eq!(picks[0].owner.as_deref(), Some("bob"));
        assert_eq!(picks[0].instance(), "bob");
        let message = picks[0].route_message().unwrap();
        assert_eq!(message["reasons"][0]["code"], "free");
        assert_eq!(message["redirectedFrom"], "alice");
        assert!(Pick::direct("me/papers", None).route_message().is_none());
    }

    #[test]
    fn no_route_says_why() {
        assert!(no_route_message("x", Some(NoRoute::NotTakingRequests)).contains("turn that on"));
        assert!(no_route_message("x", None).starts_with("No agent that serves 'x'"));
        assert!(no_route_message("x", Some(NoRoute::NobodyAvailable)).contains("right now"));
    }
}
