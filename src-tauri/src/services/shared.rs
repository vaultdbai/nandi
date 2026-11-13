//! What's shared with me (team registries + agents online at the relay) and
//! what I share (agents accepting delegation or shared to a team), with call
//! counts from the `delegations` history.
//!
//! Every source is optional: the team manager and the relay are `None` (or
//! answer errors) until configured, and a source that's unavailable is
//! skipped rather than failing the whole list.
//!
//! `list_capability_providers` turns both lists around: per capability,
//! everyone who serves it, in the order delegation would try them (my own
//! first, then the relay's plan, see `a2a::route`), with the relay's
//! reasons and my pin for it. The track record shown is mine (my
//! delegations to them), never another member's.
//!
//! Trust applies to both: agents I blocked never show, and a
//! member whose delegations I paused isn't offered as a provider.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::a2a::delegator::{self, USER_REQUESTER};
use super::a2a::{route, TaskStatus};
use super::domain::agent_card::{card_from_agent, OwnerRef};
use super::domain::{Agent, AgentCard, EntityKind};
use super::network;
use super::relay;
use super::relay::protocol::{RoutePlan, RouteReason, MAX_CANDIDATES};
use super::repo_store::RepoStore;
use super::repos::SingletonRepository;
use super::team::trust::TrustFilter;
use super::team::{TeamManager, TeamShare};
use super::trust::{self, TrustPolicy, TrustRule};
use super::AppState;
use crate::db::{now_ms, Db};
use crate::model::Timestamp;

/// Call counts cover the last 30 days
pub const CALL_WINDOW_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// My finished delegations to a provider needed before their success
/// rate shows
pub const MIN_SAMPLES: u32 = 3;

/// My track record with a provider (outbound delegations). Only mine:
/// other members' records stay at the relay.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStats {
    pub calls: u32,
    pub succeeded: u32,
    /// Failed or timed out (cancelled tasks don't count either way)
    pub failed: u32,
    /// Mean duration of the succeeded ones
    pub avg_duration_ms: Option<u32>,
}

impl ProviderStats {
    /// Succeeded / finished, once there are `MIN_SAMPLES` finished tasks
    pub fn success_rate(&self) -> Option<f64> {
        let finished = self.succeeded + self.failed;
        (finished >= MIN_SAMPLES).then(|| f64::from(self.succeeded) / f64::from(finished))
    }
}

/// Why a card can't take a task right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum Unavailable {
    /// Doesn't serve the capability
    NotServing,
    /// `accepts_delegation` is off
    Closed,
    /// Running as many delegated tasks as it allows
    Full,
    /// It's the requester
    IsRequester,
    /// Not online at the relay (known only from a team registry)
    Offline,
    /// Online, but the relay doesn't pick them now (failing lately, or
    /// missing what the request needs)
    NotNow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ShareSource {
    Team,
    Relay,
}

/// An agent card with how it's shared and how much it's used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct SharedAgent {
    pub card: AgentCard,
    /// Where it was found (with me) or how it's exposed (by me)
    pub sources: Vec<ShareSource>,
    pub team_ids: Vec<String>,
    /// Delegations in the last 30 days
    pub call_count: u32,
    pub last_called_at: Option<Timestamp>,
}

/// One agent that serves a capability, as the "who gets the work" view
/// shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ProviderOption {
    pub card: AgentCard,
    /// On this instance
    pub mine: bool,
    /// Callable now: mine, or online at the relay
    pub online: bool,
    /// Its owner is in one of my teams
    pub teammate: bool,
    pub sources: Vec<ShareSource>,
    pub team_ids: Vec<String>,
    /// My delegations to it in the last 30 days
    pub stats: ProviderStats,
    /// Succeeded / finished, once there's enough history
    pub success_rate: Option<f64>,
    /// 1 = auto would pick it; `None` while it can't take tasks
    pub rank: Option<u32>,
    pub unavailable: Option<Unavailable>,
    /// Why the relay picked them where it did (other members' only)
    pub reasons: Vec<RouteReason>,
}

/// Everyone serving one capability, best first, and my pin for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityProviders {
    pub capability: String,
    pub providers: Vec<ProviderOption>,
    /// The card ID I pinned (may be offline or gone)
    pub pinned: Option<String>,
    /// Who an auto-picked task goes to first
    pub auto_choice: Option<String>,
}

/// Delegations to one target in the window: count and the latest start.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CallStats {
    pub count: u32,
    pub last_at: Option<Timestamp>,
}

impl CallStats {
    fn add(&mut self, other: CallStats) {
        self.count += other.count;
        self.last_at = match (self.last_at, other.last_at) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
    }
}

// ---------------------------------------------------------------------------
// Pure merges (tested directly)
// ---------------------------------------------------------------------------

/// Merges team-registry cards (`(team_id, card)`) and relay cards by card
/// ID. Cards from this instance are left out (they're mine). A relay copy
/// (live load) replaces a registry one. Sorted by name, then ID.
pub fn merge_shared_with_me(
    team_cards: Vec<(String, AgentCard)>,
    relay_cards: Vec<AgentCard>,
    local_instance_id: &str,
    calls: &HashMap<String, CallStats>,
) -> Vec<SharedAgent> {
    let mut merged: BTreeMap<String, SharedAgent> = BTreeMap::new();
    for (team_id, card) in team_cards {
        if card.instance_id == local_instance_id {
            continue;
        }
        let entry = merged
            .entry(card.id.clone())
            .or_insert_with(|| SharedAgent {
                card,
                sources: Vec::new(),
                team_ids: Vec::new(),
                call_count: 0,
                last_called_at: None,
            });
        push_unique(&mut entry.sources, ShareSource::Team);
        push_unique(&mut entry.team_ids, team_id);
    }
    for card in relay_cards {
        if card.instance_id == local_instance_id {
            continue;
        }
        match merged.get_mut(&card.id) {
            Some(entry) => {
                // Keep a known owner if the relay copy lacks one
                let owner = if card.owner.github_handle.is_empty() {
                    entry.card.owner.clone()
                } else {
                    card.owner.clone()
                };
                entry.card = card.with_owner(owner);
                push_unique(&mut entry.sources, ShareSource::Relay);
            }
            None => {
                merged.insert(
                    card.id.clone(),
                    SharedAgent {
                        card,
                        sources: vec![ShareSource::Relay],
                        team_ids: Vec::new(),
                        call_count: 0,
                        last_called_at: None,
                    },
                );
            }
        }
    }
    let mut list: Vec<SharedAgent> = merged
        .into_values()
        .map(|mut shared| {
            let stats = calls.get(&shared.card.id).copied().unwrap_or_default();
            shared.call_count = stats.count;
            shared.last_called_at = stats.last_at;
            shared
        })
        .collect();
    sort_by_name(&mut list);
    list
}

/// My agents that accept delegation (exposed at the relay) or are shared to
/// a team. `calls` is keyed by card ID or agent ID (history rows may use
/// either); both are summed.
pub fn merge_shared_by_me(
    cards: Vec<AgentCard>,
    shares: &[TeamShare],
    calls: &HashMap<String, CallStats>,
) -> Vec<SharedAgent> {
    let mut list = Vec::new();
    for card in cards {
        let agent_id = card.agent_id().to_string();
        let mut team_ids: Vec<String> = Vec::new();
        for share in shares.iter().filter(|s| s.agent_id == agent_id) {
            push_unique(&mut team_ids, share.team_id.clone());
        }
        let mut sources = Vec::new();
        if !team_ids.is_empty() {
            sources.push(ShareSource::Team);
        }
        if card.accepts_delegation {
            sources.push(ShareSource::Relay);
        }
        if sources.is_empty() {
            continue;
        }
        let mut stats = calls.get(&card.id).copied().unwrap_or_default();
        if agent_id != card.id {
            if let Some(by_agent) = calls.get(&agent_id) {
                stats.add(*by_agent);
            }
        }
        list.push(SharedAgent {
            card,
            sources,
            team_ids,
            call_count: stats.count,
            last_called_at: stats.last_at,
        });
    }
    sort_by_name(&mut list);
    list
}

fn option(card: AgentCard, mine: bool, online: bool) -> ProviderOption {
    ProviderOption {
        card,
        mine,
        online,
        teammate: false,
        sources: Vec::new(),
        team_ids: Vec::new(),
        stats: ProviderStats::default(),
        success_rate: None,
        rank: None,
        unavailable: None,
        reasons: Vec::new(),
    }
}

/// Whether `card` could take a `capability` task from `requester`.
pub fn availability(
    card: &AgentCard,
    capability: &str,
    requester: &str,
) -> Result<(), Unavailable> {
    if !card.serves(capability) {
        Err(Unavailable::NotServing)
    } else if card.id == requester {
        Err(Unavailable::IsRequester)
    } else if !card.accepts_delegation {
        Err(Unavailable::Closed)
    } else if card.current_load >= card.max_concurrent.max(1) {
        Err(Unavailable::Full)
    } else {
        Ok(())
    }
}

/// What the overview shows besides the cards, gathered once per listing.
#[derive(Debug, Clone, Default)]
pub struct OverviewData {
    /// Capability → pinned card ID (`settings.yaml`)
    pub pins: BTreeMap<String, String>,
    /// Lowercase GitHub handles of my teams' members
    pub teammates: HashSet<String>,
    /// My delegations, by card ID
    pub stats: HashMap<String, ProviderStats>,
    /// The relay's plan per capability (none without a relay)
    pub plans: HashMap<String, RoutePlan>,
}

impl OverviewData {
    /// Pins from the gold copy, teammates from my teams, my history of the
    /// last 30 days. A source that fails is left empty.
    pub async fn gather(store: &RepoStore, db: &Db, teams: Option<&dyn TeamManager>) -> Self {
        let pins = store.read(|repo| repo.settings().preferred_providers().clone());
        let teammates = match teams {
            Some(teams) => match teams.list_teams().await {
                Ok(teams) => teams
                    .iter()
                    .flat_map(|team| &team.members)
                    .map(|member| member.github_handle.to_lowercase())
                    .filter(|handle| !handle.is_empty())
                    .collect(),
                Err(e) => {
                    log::debug!("Providers without teammates: {e}");
                    HashSet::new()
                }
            },
            None => HashSet::new(),
        };
        let stats = load_stats(db, window_start()).unwrap_or_else(|e| {
            log::debug!("Providers without history: {e}");
            HashMap::new()
        });
        Self {
            pins,
            teammates,
            stats,
            plans: HashMap::new(),
        }
    }

    fn is_teammate(&self, card: &AgentCard) -> bool {
        let handle = card.owner.github_handle.to_lowercase();
        !handle.is_empty() && self.teammates.contains(&handle)
    }
}

/// The relay's plan's candidate for `card`, if the plan has it free
fn free_in(plan: Option<&RoutePlan>, card_id: &str) -> bool {
    plan.and_then(|plan| plan.candidates.iter().find(|c| c.target == card_id))
        .is_some_and(|c| {
            c.reasons
                .iter()
                .any(|reason| matches!(reason, RouteReason::Free { .. }))
        })
}

/// Who an auto-picked `capability` task tries, in order (card IDs): a
/// pinned card of another member that the relay has free, then my own
/// (they run here, with no relay), then the relay's plan
pub fn auto_order(
    mine: &[AgentCard],
    capability: &str,
    requester: &str,
    pin: Option<&str>,
    plan: Option<&RoutePlan>,
) -> Vec<String> {
    let mut order = Vec::new();
    if let Some(pin) = pin.filter(|pin| free_in(plan, pin)) {
        order.push(pin.to_string());
    }
    order.extend(delegator::local_order(mine, capability, requester, pin));
    for candidate in plan
        .map(|plan| plan.candidates.as_slice())
        .unwrap_or_default()
    {
        if !order.contains(&candidate.target) {
            order.push(candidate.target.clone());
        }
    }
    order
}

/// Per capability (sorted; just `only` when given): my cards and the ones
/// shared with me, in the order an auto-picked task from `requester`
/// tries them (see [`auto_order`]), with the relay's reasons for the
/// others'. Unavailable ones follow, by name.
pub fn capability_overview(
    mine: Vec<AgentCard>,
    others: Vec<SharedAgent>,
    data: &OverviewData,
    requester: &str,
    only: Option<&str>,
) -> Vec<CapabilityProviders> {
    let mut options: Vec<ProviderOption> = mine
        .iter()
        .cloned()
        .map(|card| option(card, true, true))
        .collect();
    options.extend(others.into_iter().map(|shared| {
        let online = shared.sources.contains(&ShareSource::Relay);
        let mut other = option(shared.card, false, online);
        other.sources = shared.sources;
        other.team_ids = shared.team_ids;
        other
    }));

    let mut capabilities: Vec<String> = match only {
        // A capability nobody serves still answers, with no providers
        Some(only) => vec![only.to_string()],
        None => options
            .iter()
            .flat_map(|option| option.card.capabilities.iter().map(|c| c.name.clone()))
            .collect(),
    };
    capabilities.sort();
    capabilities.dedup();

    capabilities
        .into_iter()
        .map(|capability| {
            let pin = data.pins.get(&capability).map(String::as_str);
            let plan = data.plans.get(&capability);
            let order = auto_order(&mine, &capability, requester, pin, plan);
            let mut providers: Vec<ProviderOption> = options
                .iter()
                .filter(|option| option.card.serves(&capability))
                .map(|option| {
                    let mut option = option.clone();
                    let stats = data.stats.get(&option.card.id).copied().unwrap_or_default();
                    option.stats = stats;
                    option.success_rate = stats.success_rate();
                    option.teammate = !option.mine && data.is_teammate(&option.card);
                    option.reasons = plan
                        .and_then(|plan| {
                            plan.candidates.iter().find(|c| c.target == option.card.id)
                        })
                        .map(|c| c.reasons.clone())
                        .unwrap_or_default();
                    option.rank = order
                        .iter()
                        .position(|id| *id == option.card.id)
                        .map(|i| i as u32 + 1);
                    option.unavailable = if option.rank.is_some() {
                        None
                    } else if !option.online {
                        Some(Unavailable::Offline)
                    } else {
                        // Online and open, but not in the relay's plan
                        Some(
                            availability(&option.card, &capability, requester)
                                .err()
                                .unwrap_or(Unavailable::NotNow),
                        )
                    };
                    option
                })
                .collect();
            providers.sort_by(|a, b| match (a.rank, b.rank) {
                (Some(a), Some(b)) => a.cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a
                    .card
                    .name
                    .to_lowercase()
                    .cmp(&b.card.name.to_lowercase())
                    .then_with(|| a.card.id.cmp(&b.card.id)),
            });
            CapabilityProviders {
                pinned: pin.map(str::to_string),
                auto_choice: providers
                    .first()
                    .filter(|p| p.rank.is_some())
                    .map(|p| p.card.id.clone()),
                capability,
                providers,
            }
        })
        .collect()
}

/// Leaves out the agents I blocked and, with `skip_paused`, every agent of
/// a member whose delegations I paused
pub fn apply_trust(
    list: Vec<SharedAgent>,
    filter: &TrustFilter,
    skip_paused: bool,
) -> Vec<SharedAgent> {
    list.into_iter()
        .filter(|shared| {
            let card = &shared.card;
            !filter.is_blocked(&card.instance_id, card.agent_id())
                && !(skip_paused && filter.is_paused(&card.instance_id))
        })
        .collect()
}

fn push_unique<T: PartialEq>(items: &mut Vec<T>, item: T) {
    if !items.contains(&item) {
        items.push(item);
    }
}

fn sort_by_name(list: &mut [SharedAgent]) {
    list.sort_by(|a, b| {
        a.card
            .name
            .to_lowercase()
            .cmp(&b.card.name.to_lowercase())
            .then_with(|| a.card.id.cmp(&b.card.id))
    });
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

/// Every team's registry cards as `(team_id, card)`. A team whose registry
/// can't be read is skipped; when the teams can't be listed, it's empty.
pub async fn team_cards(manager: &dyn TeamManager) -> Vec<(String, AgentCard)> {
    let teams = match manager.list_teams().await {
        Ok(teams) => teams,
        Err(error) => {
            log::debug!("Shared views: teams unavailable: {error}");
            return Vec::new();
        }
    };
    let mut cards = Vec::new();
    for team in teams {
        match manager.team_agents(&team.id).await {
            Ok(agents) => cards.extend(agents.into_iter().map(|card| (team.id.clone(), card))),
            Err(error) => {
                let team_id = &team.id;
                log::debug!("Shared views: team {team_id} unavailable: {error}");
            }
        }
    }
    cards
}

/// My shares to teams; empty without a manager or when it fails.
async fn my_team_shares(state: &AppState) -> Vec<TeamShare> {
    let Some(manager) = state.team_manager() else {
        return Vec::new();
    };
    manager.shares_by_me().await.unwrap_or_else(|error| {
        log::debug!("Shared views: shares unavailable: {error}");
        Vec::new()
    })
}

/// Delegations in `direction` since `since_ms`, grouped by `target_agent`.
pub fn call_stats(
    db: &Db,
    direction: &str,
    since_ms: i64,
) -> Result<HashMap<String, CallStats>, String> {
    let conn = db.conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT target_agent, COUNT(*), MAX(started_at) FROM delegations
              WHERE direction = ?1 AND started_at >= ?2
              GROUP BY target_agent",
        )
        .map_err(|e| format!("Database error: {e}"))?;
    let rows = stmt
        .query_map(params![direction, since_ms], |row| {
            let target: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            let last: Option<i64> = row.get(2)?;
            Ok((
                target,
                CallStats {
                    count: u32::try_from(count).unwrap_or(u32::MAX),
                    last_at: last.map(|ms| ms as Timestamp),
                },
            ))
        })
        .map_err(|e| format!("Database error: {e}"))?;
    rows.collect::<rusqlite::Result<HashMap<_, _>>>()
        .map_err(|e| format!("Database error: {e}"))
}

/// The owner of my cards: NANDI.md's (onboarding's name and slug, as the
/// capability index keeps it); none before onboarding
fn my_owner(state: &AppState) -> OwnerRef {
    state
        .capability_index()
        .map(|index| {
            index
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .owner()
                .clone()
        })
        .unwrap_or_default()
}

/// Cards for every local agent
fn local_cards(state: &AppState, instance_id: &str) -> Vec<AgentCard> {
    let agents: Vec<Agent> = state.store().read(|repo| repo.agents().cloned().collect());
    let relay_url = relay::configured_url(state);
    let owner = my_owner(state);
    agents
        .iter()
        .map(|agent| {
            let endpoint = relay_url
                .as_deref()
                .map(|url| network::agent_endpoint(url, instance_id, agent.id().as_str()))
                .unwrap_or_default();
            card_from_agent(agent, instance_id, &endpoint).with_owner(owner.clone())
        })
        .collect()
}

fn window_start() -> i64 {
    now_ms() - CALL_WINDOW_MS
}

// ---------------------------------------------------------------------------
// Service functions
// ---------------------------------------------------------------------------

pub async fn list_agents_shared_with_me(state: &AppState) -> Result<Vec<SharedAgent>, String> {
    let instance_id = network::local_instance_id(state.db())?;
    let team = match state.team_manager() {
        Some(manager) => team_cards(manager.as_ref()).await,
        None => Vec::new(),
    };
    let online = relay::list_online_agents(state, None)
        .await
        .unwrap_or_else(|error| {
            log::debug!("Shared views: relay unavailable: {error}");
            Vec::new()
        });
    let calls = call_stats(state.db(), "outbound", window_start())?;
    let trust = TrustFilter::load(state.db())?;
    Ok(apply_trust(
        merge_shared_with_me(team, online, &instance_id, &calls),
        &trust,
        false,
    ))
}

pub async fn list_agents_shared_by_me(state: &AppState) -> Result<Vec<SharedAgent>, String> {
    let instance_id = network::local_instance_id(state.db())?;
    let cards = local_cards(state, &instance_id);
    let shares = my_team_shares(state).await;
    let calls = call_stats(state.db(), "inbound", window_start())?;
    Ok(merge_shared_by_me(cards, &shares, &calls))
}

/// Who serves each capability (or just `capability`), in the order
/// delegation tries them, with the relay's reasons and my pin
pub async fn list_capability_providers(
    state: &AppState,
    capability: Option<&str>,
) -> Result<Vec<CapabilityProviders>, String> {
    let instance_id = network::local_instance_id(state.db())?;
    let mine = local_cards(state, &instance_id);
    let trust = TrustFilter::load(state.db())?;
    let others = apply_trust(list_agents_shared_with_me(state).await?, &trust, true);
    let mut data = OverviewData::gather(
        state.store(),
        state.db(),
        state.team_manager().map(|manager| manager.as_ref()),
    )
    .await;
    // The relay plans each capability someone else serves online
    let mut remote: Vec<String> = others
        .iter()
        .filter(|shared| shared.sources.contains(&ShareSource::Relay))
        .flat_map(|shared| shared.card.capabilities.iter().map(|c| c.name.clone()))
        .filter(|name| capability.is_none_or(|only| only.trim() == name))
        .collect();
    remote.sort();
    remote.dedup();
    let plans = futures_util::future::join_all(remote.into_iter().map(|name| async move {
        let mut hints = route::hints(state.store(), state.db(), &name, &HashSet::new());
        hints.max_candidates = Some(MAX_CANDIDATES);
        let query = route::query(&name, None, hints);
        let plan = route::plan(state.relay_client(), &query).await;
        (name, plan)
    }))
    .await;
    for (name, plan) in plans {
        match plan {
            Ok(plan) => {
                data.plans.insert(name, plan);
            }
            Err(e) => log::debug!("Providers of '{name}' without the relay's plan: {e}"),
        }
    }
    let requester = AgentCard::card_id(&instance_id, USER_REQUESTER);
    let only = capability.map(str::trim).filter(|c| !c.is_empty());
    Ok(capability_overview(mine, others, &data, &requester, only))
}

/// Outbound delegations since `since_ms`, by provider card ID.
pub fn load_stats(db: &Db, since_ms: i64) -> Result<HashMap<String, ProviderStats>, String> {
    let conn = db.conn()?;
    let mut stmt = conn
        .prepare(
            "SELECT target_agent, status, started_at, ended_at FROM delegations
              WHERE direction = 'outbound' AND started_at >= ?1 AND target_agent != ''",
        )
        .map_err(|e| format!("Database error: {e}"))?;
    let rows = stmt
        .query_map(params![since_ms], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .map_err(|e| format!("Database error: {e}"))?;
    let mut totals: HashMap<String, (ProviderStats, u64)> = HashMap::new();
    for row in rows {
        let (target, status, started, ended) = row.map_err(|e| format!("Database error: {e}"))?;
        let (stats, total_ms) = totals.entry(target).or_default();
        stats.calls += 1;
        match TaskStatus::parse(&status) {
            TaskStatus::Completed => {
                stats.succeeded += 1;
                if let Some(ended) = ended {
                    *total_ms += u64::try_from(ended - started).unwrap_or(0);
                }
            }
            TaskStatus::Failed | TaskStatus::TimedOut => stats.failed += 1,
            TaskStatus::Running | TaskStatus::Cancelled => {}
        }
    }
    Ok(totals
        .into_iter()
        .map(|(target, (mut stats, total_ms))| {
            if stats.succeeded > 0 {
                let avg = total_ms / u64::from(stats.succeeded);
                stats.avg_duration_ms = Some(u32::try_from(avg).unwrap_or(u32::MAX));
            }
            (target, stats)
        })
        .collect())
}

/// Pins `card_id` for `capability` (my settings.yaml), or unpins it
pub async fn set_provider_pin(
    state: &AppState,
    capability: &str,
    card_id: Option<&str>,
) -> Result<(), String> {
    let capability = capability.trim();
    if capability.is_empty() {
        return Err("Choose a capability to pin".to_string());
    }
    let card_id = card_id.map(str::trim).filter(|id| !id.is_empty());
    let store = state.store();
    let mut settings = store.read(|repo| repo.settings().clone());
    if settings.preferred_provider(capability) == card_id {
        return Ok(());
    }
    settings
        .set_preferred_provider(capability, card_id)
        .map_err(|e| e.to_string())?;
    store.repos().settings.save(&settings)?;
    store.commit(EntityKind::Settings, "settings")?;
    Ok(())
}

pub async fn list_trust_policies(state: &AppState) -> Result<Vec<TrustRule>, String> {
    trust::list_policies(state.db())
}

pub async fn set_trust_policy(
    state: &AppState,
    requester: &str,
    provider_agent: &str,
    policy: TrustPolicy,
) -> Result<(), String> {
    let requester = requester.trim();
    if requester.is_empty() {
        return Err("Requester is required (a card ID or *)".to_string());
    }
    // Policies are keyed by local agent ID; accept a local card ID too
    let instance_id = network::local_instance_id(state.db())?;
    let prefix = format!("{instance_id}/");
    let provider_agent = provider_agent
        .strip_prefix(&prefix)
        .unwrap_or(provider_agent);
    state.store().require_agent(provider_agent)?;
    trust::set_policy(state.db(), requester, provider_agent, policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    use futures_util::future::BoxFuture;

    use crate::services::domain::AgentId;
    use crate::services::repos::Repository;
    use crate::services::team::Team;

    fn card(instance: &str, agent: &str, name: &str) -> AgentCard {
        let skill = format!(
            "---\nname: {name}\nserves: [{{name: find-papers}}]\naccepts_delegation: true\n---\n"
        );
        let agent_def = Agent::from_skill_md(AgentId::parse(agent).unwrap(), &skill).unwrap();
        card_from_agent(&agent_def, instance, "").with_owner(OwnerRef {
            github_handle: format!("{instance}-owner"),
            display_name: format!("{instance} owner"),
        })
    }

    fn team(id: &str) -> Team {
        Team {
            id: id.to_string(),
            name: id.to_string(),
            github_org: id.to_string(),
            registry_repo: format!("github.com/{id}/nandi-registry"),
            members: Vec::new(),
            created_at: 0.0,
            source: crate::services::team::TeamSource::git(&format!(
                "https://github.com/{id}/nandi-registry.git"
            )),
        }
    }

    /// Teams `acme` (sharing `remote/papers`) and `broken` (an unreadable
    /// registry)
    struct FakeTeams;

    impl TeamManager for FakeTeams {
        fn list_teams(&self) -> BoxFuture<'_, Result<Vec<Team>, String>> {
            Box::pin(async { Ok(vec![team("acme"), team("broken")]) })
        }
        fn team_agents<'a>(
            &'a self,
            team_id: &'a str,
        ) -> BoxFuture<'a, Result<Vec<AgentCard>, String>> {
            Box::pin(async move {
                match team_id {
                    "acme" => Ok(vec![card("remote", "papers", "Papers")]),
                    _ => Err("registry not synced".to_string()),
                }
            })
        }
        fn shares_by_me(&self) -> BoxFuture<'_, Result<Vec<TeamShare>, String>> {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    #[tokio::test]
    async fn a_team_shared_agent_shows_up_with_its_team() {
        let cards = team_cards(&FakeTeams).await;
        assert_eq!(cards.len(), 1, "the broken registry is skipped");
        let list = merge_shared_with_me(cards, Vec::new(), "me", &HashMap::new());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].card.id, "remote/papers");
        assert_eq!(list[0].sources, vec![ShareSource::Team]);
        assert_eq!(list[0].team_ids, vec!["acme".to_string()]);
        assert_eq!(list[0].card.owner.github_handle, "remote-owner");
    }

    #[test]
    fn team_and_relay_copies_merge_by_card_id_and_mine_are_left_out() {
        let calls = HashMap::from([(
            "remote/papers".to_string(),
            CallStats {
                count: 3,
                last_at: Some(42.0),
            },
        )]);
        let mut live = card("remote", "papers", "Papers");
        live.current_load = 2;
        let list = merge_shared_with_me(
            vec![
                ("acme".to_string(), card("remote", "papers", "Papers")),
                ("beta".to_string(), card("remote", "papers", "Papers")),
                ("acme".to_string(), card("me", "writer", "Writer")),
            ],
            vec![
                live,
                card("other", "atlas", "Atlas"),
                card("me", "local", "Local"),
            ],
            "me",
            &calls,
        );
        let ids: Vec<&str> = list.iter().map(|s| s.card.id.as_str()).collect();
        assert_eq!(ids, vec!["other/atlas", "remote/papers"]);
        let papers = &list[1];
        assert_eq!(papers.sources, vec![ShareSource::Team, ShareSource::Relay]);
        assert_eq!(papers.team_ids, vec!["acme", "beta"]);
        assert_eq!(papers.card.current_load, 2, "the live relay copy wins");
        assert_eq!((papers.call_count, papers.last_called_at), (3, Some(42.0)));
        assert_eq!(list[0].sources, vec![ShareSource::Relay]);
        assert_eq!(list[0].call_count, 0);
    }

    #[test]
    fn shared_by_me_sums_calls_by_card_id_and_agent_id() {
        let mut private = card("me", "private", "Private");
        private.accepts_delegation = false;
        let mut team_only = card("me", "writer", "Writer");
        team_only.accepts_delegation = false;
        let calls = HashMap::from([
            (
                "me/papers".to_string(),
                CallStats {
                    count: 2,
                    last_at: Some(10.0),
                },
            ),
            (
                "papers".to_string(),
                CallStats {
                    count: 1,
                    last_at: Some(20.0),
                },
            ),
        ]);
        let shares = vec![TeamShare {
            team_id: "acme".to_string(),
            agent_id: "writer".to_string(),
            shared_at: 0.0,
        }];
        let list = merge_shared_by_me(
            vec![private, team_only, card("me", "papers", "Papers")],
            &shares,
            &calls,
        );
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].card.id, "me/papers");
        assert_eq!(list[0].sources, vec![ShareSource::Relay]);
        assert_eq!(
            (list[0].call_count, list[0].last_called_at),
            (3, Some(20.0))
        );
        assert_eq!(list[1].card.id, "me/writer");
        assert_eq!(list[1].sources, vec![ShareSource::Team]);
        assert_eq!(list[1].team_ids, vec!["acme"]);
    }

    fn insert_delegation(db: &Db, id: &str, direction: &str, target: &str, started_at: i64) {
        db.conn()
            .unwrap()
            .execute(
                "INSERT INTO delegations
                   (id, direction, capability, target_agent, requester_agent, status, started_at)
                 VALUES (?1, ?2, 'find-papers', ?3, 'x/y', 'completed', ?4)",
                params![id, direction, target, started_at],
            )
            .unwrap();
    }

    #[test]
    fn call_stats_count_the_window_per_direction() {
        let db = Db::in_memory();
        let now = now_ms();
        insert_delegation(&db, "1", "inbound", "papers", now - 1000);
        insert_delegation(&db, "2", "inbound", "papers", now - 500);
        insert_delegation(&db, "3", "inbound", "papers", now - CALL_WINDOW_MS - 1000);
        insert_delegation(&db, "4", "outbound", "remote/papers", now);
        let inbound = call_stats(&db, "inbound", now - CALL_WINDOW_MS).unwrap();
        assert_eq!(inbound.len(), 1);
        assert_eq!(inbound["papers"].count, 2);
        assert_eq!(inbound["papers"].last_at, Some((now - 500) as Timestamp));
        let outbound = call_stats(&db, "outbound", now - CALL_WINDOW_MS).unwrap();
        assert_eq!(outbound["remote/papers"].count, 1);
    }

    #[tokio::test]
    async fn shared_by_me_lists_delegating_agents_with_inbound_calls() {
        let state = AppState::for_tests(vec![]);
        for (id, frontmatter) in [
            (
                "papers",
                "serves: [{name: find-papers}]\naccepts_delegation: true\n",
            ),
            ("quiet", ""),
        ] {
            let agent = Agent::from_skill_md(
                AgentId::parse(id).unwrap(),
                &format!("---\nname: {id}\n{frontmatter}---\nHelp.\n"),
            )
            .unwrap();
            state.store().repos().agents.save(&agent).unwrap();
            state.store().commit(EntityKind::Agent, id).unwrap();
        }
        let instance = network::local_instance_id(state.db()).unwrap();
        let card_id = format!("{instance}/papers");
        insert_delegation(state.db(), "a", "inbound", &card_id, now_ms());
        insert_delegation(state.db(), "b", "inbound", "papers", now_ms());

        let list = list_agents_shared_by_me(&state).await.unwrap();
        let ids: Vec<&str> = list.iter().map(|s| s.card.agent_id()).collect();
        assert!(
            ids.contains(&"papers") && !ids.contains(&"quiet"),
            "{ids:?}"
        );
        let papers = list.iter().find(|s| s.card.agent_id() == "papers").unwrap();
        assert_eq!(papers.call_count, 2);
        assert!(papers.last_called_at.is_some());

        // Onboarding's name and slug own the cards
        crate::services::auth::set_user_slug(&state, "ada-l".into())
            .await
            .unwrap();
        crate::services::auth::set_display_name(&state, "Ada".into())
            .await
            .unwrap();
        state.gateway().unwrap().sync().await;
        let list = list_agents_shared_by_me(&state).await.unwrap();
        assert!(list.iter().all(|s| s.card.owner.github_handle == "ada-l"));

        // Without a team manager or relay, "with me" is empty, not an error
        if state.team_manager().is_none() && state.relay_client().is_none() {
            assert!(list_agents_shared_with_me(&state).await.unwrap().is_empty());
        }

        // Trust policies by agent ID or local card ID; unknown agents fail
        set_trust_policy(&state, "other/x", "papers", TrustPolicy::Allow)
            .await
            .unwrap();
        set_trust_policy(&state, "*", &card_id, TrustPolicy::Deny)
            .await
            .unwrap();
        assert!(
            set_trust_policy(&state, "other/x", "ghost", TrustPolicy::Allow)
                .await
                .is_err()
        );
        assert!(set_trust_policy(&state, " ", "papers", TrustPolicy::Allow)
            .await
            .is_err());
        let rules = list_trust_policies(&state).await.unwrap();
        assert_eq!(rules.len(), 2);
        assert!(rules.iter().all(|r| r.provider_agent == "papers"));
        state.shutdown();
    }

    /// A plan of `targets`, each free
    fn plan(targets: &[&str]) -> RoutePlan {
        RoutePlan {
            candidates: targets
                .iter()
                .map(|target| {
                    let instance = target.split('/').next().unwrap();
                    crate::services::relay::protocol::RouteCandidate {
                        target: target.to_string(),
                        instance_id: instance.to_string(),
                        name: format!("{instance} owner"),
                        handle: None,
                        agent_name: String::new(),
                        provider_kind: Default::default(),
                        online: true,
                        load: 0,
                        max_concurrent: 1,
                        wait_seconds: None,
                        redirected_from: None,
                        reasons: vec![RouteReason::Free {
                            name: format!("{instance} owner"),
                        }],
                        public_keys: None,
                    }
                })
                .collect(),
            ..RoutePlan::default()
        }
    }

    #[test]
    fn my_track_record_needs_three_finished_tasks() {
        let stats = |succeeded, failed| ProviderStats {
            calls: succeeded + failed,
            succeeded,
            failed,
            avg_duration_ms: None,
        };
        assert_eq!(stats(0, 2).success_rate(), None);
        assert_eq!(stats(2, 1).success_rate(), Some(2.0 / 3.0));
    }

    #[test]
    fn my_stats_count_outcomes_and_average_successes() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        let rows = [
            ("t1", "them/a", "completed", 0, Some(1_000)),
            ("t2", "them/a", "completed", 0, Some(3_000)),
            ("t3", "them/a", "failed", 0, Some(10)),
            ("t4", "them/a", "cancelled", 0, None),
            ("t5", "them/b", "timedOut", 0, None),
        ];
        for (id, target, status, started, ended) in rows {
            conn.execute(
                "INSERT INTO delegations (id, direction, capability, target_agent,
                   requester_agent, status, started_at, ended_at)
                 VALUES (?1, 'outbound', 'summarize', ?2, 'me/_user', ?3, ?4, ?5)",
                params![id, target, status, started, ended],
            )
            .unwrap();
        }
        drop(conn); // the in-memory pool has one connection
        let stats = load_stats(&db, 0).unwrap();
        assert_eq!(
            stats["them/a"],
            ProviderStats {
                calls: 4,
                succeeded: 2,
                failed: 1,
                avg_duration_ms: Some(2_000),
            }
        );
        assert_eq!(stats["them/b"].failed, 1);
    }

    #[test]
    fn availability_names_the_reason() {
        let requester = "me/_user";
        let mut closed = card("x", "a", "A");
        closed.accepts_delegation = false;
        assert_eq!(
            availability(&closed, "find-papers", requester),
            Err(Unavailable::Closed)
        );
        assert_eq!(
            availability(&card("x", "a", "A"), "translate", requester),
            Err(Unavailable::NotServing)
        );
        let mut full = card("x", "a", "A");
        full.current_load = full.max_concurrent.max(1);
        assert_eq!(
            availability(&full, "find-papers", requester),
            Err(Unavailable::Full)
        );
        assert_eq!(
            availability(&card("x", "a", "A"), "find-papers", requester),
            Ok(())
        );
    }

    fn shared(card: AgentCard, source: ShareSource) -> SharedAgent {
        SharedAgent {
            card,
            sources: vec![source],
            team_ids: Vec::new(),
            call_count: 0,
            last_called_at: None,
        }
    }

    #[test]
    fn blocked_agents_are_filtered_from_listings() {
        use crate::services::team::trust::set_blocked;
        let db = Db::in_memory();
        set_blocked(&db, "remote", "papers", true).unwrap();
        let filter = TrustFilter::load(&db).unwrap();
        let list = apply_trust(
            vec![
                shared(card("remote", "papers", "Papers"), ShareSource::Relay),
                shared(card("remote", "notes", "Notes"), ShareSource::Team),
                shared(card("other", "papers", "Other papers"), ShareSource::Relay),
            ],
            &filter,
            false,
        );
        let ids: Vec<&str> = list.iter().map(|s| s.card.id.as_str()).collect();
        assert_eq!(ids, ["remote/notes", "other/papers"]);
    }

    #[test]
    fn a_paused_member_is_not_offered_as_a_provider() {
        use crate::services::team::trust::{update_relationship, TrustUpdate};
        let db = Db::in_memory();
        update_relationship(
            &db,
            "far",
            &TrustUpdate {
                added_as_trusted_peer: None,
                delegations_paused: Some(true),
            },
        )
        .unwrap();
        let filter = TrustFilter::load(&db).unwrap();
        let others = vec![
            shared(card("far", "y", "Paused"), ShareSource::Relay),
            shared(card("near", "z", "Open"), ShareSource::Relay),
        ];
        // Still listed as shared with me...
        assert_eq!(apply_trust(others.clone(), &filter, false).len(), 2);
        // ...but not a provider
        let providers = apply_trust(others, &filter, true);
        let data = OverviewData {
            plans: HashMap::from([("find-papers".to_string(), plan(&["near/z"]))]),
            ..OverviewData::default()
        };
        let overview = capability_overview(
            Vec::new(),
            providers,
            &data,
            "me/_user",
            Some("find-papers"),
        );
        let ids: Vec<&str> = overview[0]
            .providers
            .iter()
            .map(|p| p.card.id.as_str())
            .collect();
        assert_eq!(ids, ["near/z"]);
        assert_eq!(overview[0].auto_choice.as_deref(), Some("near/z"));
    }

    #[test]
    fn the_overview_ranks_callable_providers_and_marks_the_rest() {
        let mine = vec![card("me", "papers", "Mine")];
        let others = vec![
            SharedAgent {
                card: card("remote", "x", "Registry only"),
                sources: vec![ShareSource::Team],
                team_ids: vec!["acme".into()],
                call_count: 0,
                last_called_at: None,
            },
            SharedAgent {
                card: card("far", "y", "Online"),
                sources: vec![ShareSource::Relay],
                team_ids: Vec::new(),
                call_count: 0,
                last_called_at: None,
            },
        ];
        let mut data = OverviewData {
            pins: BTreeMap::from([("find-papers".to_string(), "far/y".to_string())]),
            teammates: ["far-owner".to_string()].into_iter().collect(),
            stats: HashMap::new(),
            plans: HashMap::from([("find-papers".to_string(), plan(&["far/y"]))]),
        };
        let overview = capability_overview(mine.clone(), others.clone(), &data, "me/_user", None);
        assert_eq!(overview.len(), 1);
        let papers = &overview[0];
        assert_eq!(papers.capability, "find-papers");
        assert_eq!(papers.pinned.as_deref(), Some("far/y"));
        assert_eq!(papers.auto_choice.as_deref(), Some("far/y"));
        let ids: Vec<_> = papers
            .providers
            .iter()
            .map(|p| p.card.id.as_str())
            .collect();
        assert_eq!(ids, ["far/y", "me/papers", "remote/x"]);
        assert_eq!(papers.providers[0].rank, Some(1));
        assert!(papers.providers[0].teammate);
        // The relay's reasons, for the others' only
        assert_eq!(
            papers.providers[0].reasons,
            [RouteReason::Free {
                name: "far owner".into()
            }]
        );
        assert!(papers.providers[1].mine && papers.providers[1].reasons.is_empty());
        assert_eq!(papers.providers[2].unavailable, Some(Unavailable::Offline));
        assert_eq!(papers.providers[2].rank, None);

        // Without the pin my own goes first; without the relay's plan the
        // online one isn't picked now
        data.pins.clear();
        data.plans.clear();
        let overview = capability_overview(mine, others, &data, "me/_user", None);
        let papers = &overview[0];
        assert_eq!(papers.auto_choice.as_deref(), Some("me/papers"));
        let far = papers
            .providers
            .iter()
            .find(|p| p.card.id == "far/y")
            .unwrap();
        assert_eq!(
            (far.rank, far.unavailable),
            (None, Some(Unavailable::NotNow))
        );

        let none = capability_overview(Vec::new(), Vec::new(), &data, "me/_user", Some("nope"));
        assert_eq!(none.len(), 1);
        assert!(none[0].providers.is_empty());
        assert_eq!(none[0].auto_choice, None);
    }
}
