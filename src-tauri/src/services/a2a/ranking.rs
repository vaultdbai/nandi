//! Who gets a delegated task when several agents serve the capability.
//!
//! Eligible: serves the capability, accepts delegation, has room
//! (`current_load < max_concurrent`), and isn't the requester. Eligible
//! cards are ranked by, in order:
//!
//! 1. the user's pin for the capability (`settings.yaml` `delegation:`)
//! 2. not unreliable (under half of its recent tasks succeeded)
//! 3. on this instance (no relay round trip)
//! 4. owned by a teammate (a member of one of my teams)
//! 5. lower current load
//! 6. faster on average (when both have history)
//! 7. fresher heartbeat (`issued_at`)
//!
//! Stats come from the outbound `delegations` rows of the last 30 days.
//! Everything here is pure except `load_stats`.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::params;
use serde::{Deserialize, Serialize};
use specta::Type;

use super::TaskStatus;
use crate::db::{now_ms, Db};
use crate::services::domain::AgentCard;
use crate::services::repo_store::RepoStore;
use crate::services::shared::CALL_WINDOW_MS;
use crate::services::team::TeamManager;

/// Finished tasks needed before a success rate counts
pub const MIN_SAMPLES: u32 = 3;

/// Below this success rate a provider is ranked after the reliable ones
pub const UNRELIABLE_BELOW: f64 = 0.5;

/// A provider's track record with me (outbound delegations).
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

    pub fn unreliable(&self) -> bool {
        self.success_rate()
            .is_some_and(|rate| rate < UNRELIABLE_BELOW)
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

/// What ranking knows beyond the cards.
#[derive(Debug, Clone, Copy)]
pub struct RankContext<'a> {
    pub local_instance: &'a str,
    /// The requesting card's ID (never picked)
    pub requester: &'a str,
    /// The card the user pinned for this capability
    pub pinned: Option<&'a str>,
    /// Lowercase GitHub handles of my teams' members
    pub teammates: &'a HashSet<String>,
    /// By card ID
    pub stats: &'a HashMap<String, ProviderStats>,
}

impl<'a> RankContext<'a> {
    /// No pin, teammates, or history: local, load, then freshness decide
    pub fn bare(local_instance: &'a str, requester: &'a str) -> Self {
        static EMPTY_TEAM: std::sync::LazyLock<HashSet<String>> =
            std::sync::LazyLock::new(HashSet::new);
        static EMPTY_STATS: std::sync::LazyLock<HashMap<String, ProviderStats>> =
            std::sync::LazyLock::new(HashMap::new);
        Self {
            local_instance,
            requester,
            pinned: None,
            teammates: &EMPTY_TEAM,
            stats: &EMPTY_STATS,
        }
    }

    fn stats_of(&self, card: &AgentCard) -> ProviderStats {
        self.stats.get(&card.id).copied().unwrap_or_default()
    }

    pub fn is_teammate(&self, card: &AgentCard) -> bool {
        let handle = card.owner.github_handle.to_lowercase();
        !handle.is_empty() && self.teammates.contains(&handle)
    }

    /// `Less` when `a` should get the task before `b`
    fn compare(&self, a: &AgentCard, b: &AgentCard) -> Ordering {
        let pinned = |card: &AgentCard| self.pinned != Some(card.id.as_str());
        let unreliable = |card: &AgentCard| self.stats_of(card).unreliable();
        let remote = |card: &AgentCard| card.instance_id != self.local_instance;
        let stranger = |card: &AgentCard| !self.is_teammate(card);
        let speed = match (
            self.stats_of(a).avg_duration_ms,
            self.stats_of(b).avg_duration_ms,
        ) {
            (Some(a), Some(b)) => a.cmp(&b),
            _ => Ordering::Equal,
        };
        pinned(a)
            .cmp(&pinned(b))
            .then(unreliable(a).cmp(&unreliable(b)))
            .then(remote(a).cmp(&remote(b)))
            .then(stranger(a).cmp(&stranger(b)))
            .then(a.current_load.cmp(&b.current_load))
            .then(speed)
            .then(
                b.issued_at
                    .partial_cmp(&a.issued_at)
                    .unwrap_or(Ordering::Equal),
            )
    }
}

/// Everything ranking reads besides the cards, gathered once per pick.
#[derive(Debug, Clone, Default)]
pub struct RankData {
    /// Capability → pinned card ID (`settings.yaml`)
    pub pins: BTreeMap<String, String>,
    pub teammates: HashSet<String>,
    pub stats: HashMap<String, ProviderStats>,
}

impl RankData {
    /// Pins from the gold copy, teammates from my teams, stats from the
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
                    log::debug!("Ranking without teammates: {e}");
                    HashSet::new()
                }
            },
            None => HashSet::new(),
        };
        let stats = load_stats(db, now_ms() - CALL_WINDOW_MS).unwrap_or_else(|e| {
            log::debug!("Ranking without history: {e}");
            HashMap::new()
        });
        Self {
            pins,
            teammates,
            stats,
        }
    }

    pub fn context<'a>(
        &'a self,
        local_instance: &'a str,
        requester: &'a str,
        capability: &str,
    ) -> RankContext<'a> {
        RankContext {
            local_instance,
            requester,
            pinned: self.pins.get(capability).map(String::as_str),
            teammates: &self.teammates,
            stats: &self.stats,
        }
    }
}

/// The cards that could take a `capability` task, best first.
pub fn rank_providers<'c>(
    cards: &'c [AgentCard],
    capability: &str,
    ctx: &RankContext<'_>,
) -> Vec<&'c AgentCard> {
    let mut eligible: Vec<&AgentCard> = cards
        .iter()
        .filter(|card| availability(card, capability, ctx.requester).is_ok())
        .collect();
    eligible.sort_by(|a, b| ctx.compare(a, b));
    eligible
}

/// The best provider for `capability`, or `None`.
pub fn choose_provider<'c>(
    cards: &'c [AgentCard],
    capability: &str,
    ctx: &RankContext<'_>,
) -> Option<&'c AgentCard> {
    rank_providers(cards, capability, ctx).into_iter().next()
}

/// Whether a failed attempt may go to the next provider. Only when the
/// provider did no work: it rejected the task (trust, closed, full), the
/// relay couldn't deliver it, or no result came in time.
pub fn retryable(status: TaskStatus, did_work: bool) -> bool {
    match status {
        TaskStatus::TimedOut => true,
        TaskStatus::Failed => !did_work,
        _ => false,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::domain::agent_card::OwnerRef;
    use crate::services::domain::capability::ServesCapability;

    fn card(id: &str, load: u32, issued_at: f64) -> AgentCard {
        let (instance, _) = id.split_once('/').unwrap();
        AgentCard {
            id: id.into(),
            name: id.into(),
            description: String::new(),
            instance_id: instance.into(),
            endpoint: String::new(),
            capabilities: vec![ServesCapability {
                name: "summarize".into(),
                description: String::new(),
                input_schema: None,
                output_schema: None,
            }],
            accepts_delegation: true,
            max_concurrent: 4,
            current_load: load,
            owner: OwnerRef::default(),
            signature: None,
            issued_at,
        }
    }

    fn owned(mut card: AgentCard, handle: &str) -> AgentCard {
        card.owner.github_handle = handle.into();
        card
    }

    fn stats(succeeded: u32, failed: u32, avg: Option<u32>) -> ProviderStats {
        ProviderStats {
            calls: succeeded + failed,
            succeeded,
            failed,
            avg_duration_ms: avg,
        }
    }

    fn order(cards: &[AgentCard], ctx: &RankContext<'_>) -> Vec<String> {
        rank_providers(cards, "summarize", ctx)
            .into_iter()
            .map(|c| c.id.clone())
            .collect()
    }

    #[test]
    fn a_pin_wins_over_everything_while_it_is_eligible() {
        let cards = [card("me/local", 0, 9.0), card("them/pinned", 3, 1.0)];
        let mut ctx = RankContext::bare("me", "me/_user");
        ctx.pinned = Some("them/pinned");
        assert_eq!(order(&cards, &ctx), ["them/pinned", "me/local"]);

        let mut full = cards.clone();
        full[1].current_load = 4;
        assert_eq!(order(&full, &ctx), ["me/local"]);
    }

    #[test]
    fn unreliable_providers_go_last_even_when_local() {
        let cards = [card("me/flaky", 0, 1.0), card("them/solid", 0, 1.0)];
        let history = HashMap::from([
            ("me/flaky".to_string(), stats(1, 4, None)),
            ("them/solid".to_string(), stats(5, 0, None)),
        ]);
        let mut ctx = RankContext::bare("me", "me/_user");
        ctx.stats = &history;
        assert_eq!(order(&cards, &ctx), ["them/solid", "me/flaky"]);
    }

    #[test]
    fn too_little_history_is_not_unreliable() {
        assert_eq!(stats(0, 2, None).success_rate(), None);
        assert!(!stats(0, 2, None).unreliable());
        assert!(stats(1, 2, None).unreliable());
        assert!(!stats(2, 1, None).unreliable());
    }

    #[test]
    fn teammates_beat_strangers_on_the_relay() {
        let cards = [
            owned(card("x/stranger", 0, 9.0), "eve"),
            owned(card("y/teammate", 1, 1.0), "Alice"),
        ];
        let team = HashSet::from(["alice".to_string()]);
        let mut ctx = RankContext::bare("me", "me/_user");
        ctx.teammates = &team;
        assert_eq!(order(&cards, &ctx), ["y/teammate", "x/stranger"]);
    }

    #[test]
    fn faster_wins_when_load_ties() {
        let cards = [card("a/slow", 0, 9.0), card("b/fast", 0, 1.0)];
        let history = HashMap::from([
            ("a/slow".to_string(), stats(3, 0, Some(9_000))),
            ("b/fast".to_string(), stats(3, 0, Some(1_000))),
        ]);
        let mut ctx = RankContext::bare("me", "me/_user");
        ctx.stats = &history;
        assert_eq!(order(&cards, &ctx), ["b/fast", "a/slow"]);
    }

    #[test]
    fn availability_names_the_reason() {
        let requester = "me/req";
        assert_eq!(
            availability(&card("me/req", 0, 0.0), "summarize", requester),
            Err(Unavailable::IsRequester)
        );
        assert_eq!(
            availability(&card("x/a", 0, 0.0), "translate", requester),
            Err(Unavailable::NotServing)
        );
        let mut closed = card("x/a", 0, 0.0);
        closed.accepts_delegation = false;
        assert_eq!(
            availability(&closed, "summarize", requester),
            Err(Unavailable::Closed)
        );
        assert_eq!(
            availability(&card("x/a", 4, 0.0), "summarize", requester),
            Err(Unavailable::Full)
        );
        assert_eq!(
            availability(&card("x/a", 0, 0.0), "summarize", requester),
            Ok(())
        );
    }

    #[test]
    fn only_rejections_and_timeouts_are_retried() {
        assert!(retryable(TaskStatus::TimedOut, true));
        assert!(retryable(TaskStatus::Failed, false));
        assert!(!retryable(TaskStatus::Failed, true));
        assert!(!retryable(TaskStatus::Completed, false));
        assert!(!retryable(TaskStatus::Cancelled, false));
    }

    #[test]
    fn stats_count_outcomes_and_average_successes() {
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
}
