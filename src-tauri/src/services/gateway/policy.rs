//! Gateway policy: a token bucket per caller
//! (`rate_limit_per_hour`, hourly reset), the monthly budget
//! (`gateway_usage`), capability exposure (`expose` / `hide`), and
//! `require_approval`. Over a limit: `gateway://rate_limited` and a
//! `RejectTask`.
//!
//! [`evaluate`] is pure apart from the bucket it draws from; the SQLite
//! helpers below it keep `gateway_callers` and `gateway_usage`.

use std::collections::HashMap;
use std::sync::Mutex;

use chrono::{TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use strum::{EnumString, IntoStaticStr};

use super::{CallerPolicy, GatewayPolicy, GatewayUsage, GATEWAY_RATE_LIMITED};
use crate::services::a2a::task::ReasonKind;
use crate::services::events::NandiEvent;

/// The bucket refills fully over this window
pub const RATE_WINDOW_MS: i64 = 60 * 60 * 1000;

/// `gateway://rate_limited`: a caller hit the rate limit or its budget
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GatewayRateLimited {
    pub caller_instance: String,
    pub capability: Option<String>,
    /// `rate_limit` | `monthly_budget`
    pub limit: String,
    pub reason: String,
}

impl NandiEvent for GatewayRateLimited {
    const NAME: &'static str = GATEWAY_RATE_LIMITED;
}

/// What the policy says about one request
#[derive(Debug, Clone, PartialEq)]
pub enum PolicyDecision {
    /// Go ahead; `needs_approval` when the capability is in
    /// `require_approval`
    Allow { needs_approval: bool },
    Reject {
        reason: String,
        /// Why, for the requester's UI (work-model §2.3)
        kind: ReasonKind,
        /// Set when a rate limit or budget refused it (`gateway://rate_limited`)
        limit: Option<LimitKind>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumString, IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum LimitKind {
    RateLimit,
    MonthlyBudget,
}

impl LimitKind {
    /// The limit's name
    pub fn as_str(self) -> &'static str {
        self.into()
    }
}

/// Whether the gateway offers `capability`: inside `expose` (when it lists
/// any), and not in `hide`. Case-insensitive.
pub fn is_exposed(policy: &GatewayPolicy, capability: &str) -> bool {
    let listed = |list: &[String]| list.iter().any(|c| c.eq_ignore_ascii_case(capability));
    (policy.expose.is_empty() || listed(&policy.expose)) && !listed(&policy.hide)
}

/// One token bucket per caller instance: `rate_limit_per_hour` tokens,
/// refilled continuously over [`RATE_WINDOW_MS`]. A limit of 0 means no
/// limit.
#[derive(Default)]
pub struct RateLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    refilled_at: i64,
}

impl RateLimiter {
    /// Takes a token for `caller` if one is left
    pub fn try_take(&self, caller: &str, limit_per_hour: u32, now_ms: i64) -> bool {
        if limit_per_hour == 0 {
            return true;
        }
        let capacity = f64::from(limit_per_hour);
        let mut buckets = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = buckets.entry(caller.to_string()).or_insert(Bucket {
            tokens: capacity,
            refilled_at: now_ms,
        });
        let elapsed = (now_ms - bucket.refilled_at).max(0) as f64;
        bucket.tokens = (bucket.tokens + capacity * elapsed / RATE_WINDOW_MS as f64).min(capacity);
        bucket.refilled_at = now_ms;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// Everything [`evaluate`] looks at
#[derive(Debug, Clone, Copy)]
pub struct PolicyInput<'a> {
    pub policy: &'a GatewayPolicy,
    pub caller: &'a CallerPolicy,
    /// `None` for a natural-language request (the router picks)
    pub capability: Option<&'a str>,
    /// The caller's tasks this month so far
    pub used_this_month: u32,
    /// We paused delegations with the caller's instance
    /// (`team::trust::is_paused`)
    pub caller_paused: bool,
    /// Who the caller is, in words (never a device ID)
    pub caller_name: &'a str,
}

/// Checks a request in order: caller disabled, caller paused (trust),
/// exposure, the caller's
/// allowed capabilities, the monthly budget, then the rate limit (last, so
/// a refused request doesn't spend a token).
pub fn evaluate(input: PolicyInput<'_>, limiter: &RateLimiter, now_ms: i64) -> PolicyDecision {
    let reject = |kind: ReasonKind, reason: String| PolicyDecision::Reject {
        reason,
        kind,
        limit: None,
    };
    let caller = input.caller;
    let who = if input.caller_name.trim().is_empty() {
        "this Nandi"
    } else {
        input.caller_name
    };
    if caller.disabled {
        return reject(
            ReasonKind::PausedOrBlocked,
            format!("Requests from {who} are turned off"),
        );
    }
    if input.caller_paused {
        return reject(
            ReasonKind::PausedOrBlocked,
            format!("Requests from {who} are paused"),
        );
    }
    if let Some(capability) = input.capability {
        if !is_exposed(input.policy, capability) {
            return reject(
                ReasonKind::NotAccepting,
                format!("'{capability}' isn't offered by this gateway"),
            );
        }
        if !caller.allowed_capabilities.is_empty()
            && !caller
                .allowed_capabilities
                .iter()
                .any(|c| c.eq_ignore_ascii_case(capability))
        {
            return reject(
                ReasonKind::NotAccepting,
                format!("{who} may not ask for '{capability}'"),
            );
        }
    }
    if let Some(budget) = caller.monthly_task_budget {
        if input.used_this_month >= budget {
            return PolicyDecision::Reject {
                reason: format!("{who} used its {budget} requests for this month"),
                kind: ReasonKind::Failed,
                limit: Some(LimitKind::MonthlyBudget),
            };
        }
    }
    if !limiter.try_take(
        &caller.caller_instance,
        input.policy.rate_limit_per_hour,
        now_ms,
    ) {
        return PolicyDecision::Reject {
            reason: format!(
                "Rate limit reached ({} requests per hour)",
                input.policy.rate_limit_per_hour
            ),
            kind: ReasonKind::Failed,
            limit: Some(LimitKind::RateLimit),
        };
    }
    let needs_approval = input.capability.is_some_and(|capability| {
        input
            .policy
            .require_approval
            .iter()
            .any(|c| c.eq_ignore_ascii_case(capability))
    });
    PolicyDecision::Allow { needs_approval }
}

/// `YYYY-MM` (UTC) of an epoch-ms instant
pub fn month_of(ms: i64) -> String {
    Utc.timestamp_millis_opt(ms)
        .single()
        .unwrap_or_default()
        .format("%Y-%m")
        .to_string()
}

/// A caller seen for the first time: allowed, one task at a time, no budget
pub fn default_caller(caller_instance: &str, caller_owner: &str, now_ms: i64) -> CallerPolicy {
    CallerPolicy {
        caller_instance: caller_instance.to_string(),
        caller_owner: caller_owner.to_string(),
        allowed_capabilities: Vec::new(),
        max_concurrent: 1,
        monthly_task_budget: None,
        disabled: false,
        created_at: now_ms as f64,
    }
}

// ---------------------------------------------------------------------------
// gateway_callers / gateway_usage
// ---------------------------------------------------------------------------

const CALLER_COLUMNS: &str = "caller_instance, caller_owner, allowed_capabilities, \
    max_concurrent, monthly_task_budget, disabled, created_at";

fn caller_row(row: &Row) -> rusqlite::Result<CallerPolicy> {
    let allowed: String = row.get(2)?;
    Ok(CallerPolicy {
        caller_instance: row.get(0)?,
        caller_owner: row.get(1)?,
        allowed_capabilities: serde_json::from_str(&allowed).unwrap_or_default(),
        max_concurrent: row.get(3)?,
        monthly_task_budget: row.get(4)?,
        disabled: row.get::<_, i64>(5)? != 0,
        created_at: row.get::<_, i64>(6)? as f64,
    })
}

pub fn list_callers(conn: &Connection) -> rusqlite::Result<Vec<CallerPolicy>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {CALLER_COLUMNS} FROM gateway_callers ORDER BY created_at, caller_instance"
    ))?;
    let rows = stmt.query_map([], caller_row)?;
    rows.collect()
}

pub fn get_caller(
    conn: &Connection,
    caller_instance: &str,
) -> rusqlite::Result<Option<CallerPolicy>> {
    conn.query_row(
        &format!("SELECT {CALLER_COLUMNS} FROM gateway_callers WHERE caller_instance = ?1"),
        [caller_instance],
        caller_row,
    )
    .optional()
}

/// Inserts or replaces a caller's row (an existing row keeps `created_at`)
pub fn upsert_caller(conn: &Connection, policy: &CallerPolicy) -> rusqlite::Result<()> {
    let allowed =
        serde_json::to_string(&policy.allowed_capabilities).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO gateway_callers (caller_instance, caller_owner, allowed_capabilities,
             max_concurrent, monthly_task_budget, disabled, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(caller_instance) DO UPDATE SET
             caller_owner = excluded.caller_owner,
             allowed_capabilities = excluded.allowed_capabilities,
             max_concurrent = excluded.max_concurrent,
             monthly_task_budget = excluded.monthly_task_budget,
             disabled = excluded.disabled",
        params![
            policy.caller_instance,
            policy.caller_owner,
            allowed,
            policy.max_concurrent,
            policy.monthly_task_budget,
            policy.disabled,
            policy.created_at as i64,
        ],
    )?;
    Ok(())
}

/// Returns whether a row was removed
pub fn remove_caller(conn: &Connection, caller_instance: &str) -> rusqlite::Result<bool> {
    let removed = conn.execute(
        "DELETE FROM gateway_callers WHERE caller_instance = ?1",
        [caller_instance],
    )?;
    Ok(removed > 0)
}

pub fn usage_count(conn: &Connection, caller_instance: &str, month: &str) -> rusqlite::Result<u32> {
    Ok(conn
        .query_row(
            "SELECT task_count FROM gateway_usage WHERE caller_instance = ?1 AND month = ?2",
            params![caller_instance, month],
            |row| row.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

pub fn increment_usage(
    conn: &Connection,
    caller_instance: &str,
    month: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO gateway_usage (caller_instance, month, task_count) VALUES (?1, ?2, 1)
         ON CONFLICT(caller_instance, month) DO UPDATE SET task_count = task_count + 1",
        params![caller_instance, month],
    )?;
    Ok(())
}

/// A caller's use in `month`, with its budget
pub fn usage(
    conn: &Connection,
    caller_instance: &str,
    month: &str,
) -> rusqlite::Result<GatewayUsage> {
    let budget = get_caller(conn, caller_instance)?.and_then(|c| c.monthly_task_budget);
    Ok(GatewayUsage {
        caller_instance: caller_instance.to_string(),
        month: month.to_string(),
        task_count: usage_count(conn, caller_instance, month)?,
        monthly_task_budget: budget,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    const HOUR: i64 = RATE_WINDOW_MS;

    fn policy() -> GatewayPolicy {
        GatewayPolicy {
            rate_limit_per_hour: 2,
            ..GatewayPolicy::default()
        }
    }

    fn check(
        policy: &GatewayPolicy,
        caller: &CallerPolicy,
        capability: Option<&str>,
        used: u32,
        limiter: &RateLimiter,
        now: i64,
    ) -> PolicyDecision {
        let input = PolicyInput {
            policy,
            caller,
            capability,
            used_this_month: used,
            caller_paused: false,
            caller_name: "Alice's Nandi",
        };
        evaluate(input, limiter, now)
    }

    fn allowed(decision: &PolicyDecision) -> bool {
        matches!(decision, PolicyDecision::Allow { .. })
    }

    #[test]
    fn rate_limits_per_caller_and_refills_hourly() {
        let (policy, limiter) = (policy(), RateLimiter::default());
        let alice = default_caller("alice", "", 0);
        let bob = default_caller("bob", "", 0);
        assert!(allowed(&check(&policy, &alice, None, 0, &limiter, 0)));
        assert!(allowed(&check(&policy, &alice, None, 0, &limiter, 1)));
        assert!(matches!(
            check(&policy, &alice, None, 0, &limiter, 2),
            PolicyDecision::Reject {
                limit: Some(LimitKind::RateLimit),
                ..
            }
        ));
        // Another caller has its own bucket
        assert!(allowed(&check(&policy, &bob, None, 0, &limiter, 2)));
        // Half an hour refills one of two tokens
        assert!(allowed(&check(
            &policy,
            &alice,
            None,
            0,
            &limiter,
            HOUR / 2 + 2
        )));
        assert!(!allowed(&check(
            &policy,
            &alice,
            None,
            0,
            &limiter,
            HOUR / 2 + 3
        )));
        // An hour refills both (and no more)
        assert!(limiter.try_take("alice", 2, 3 * HOUR));
        assert!(limiter.try_take("alice", 2, 3 * HOUR));
        assert!(!limiter.try_take("alice", 2, 3 * HOUR));
        // 0: no limit
        assert!((0..100).all(|i| limiter.try_take("carol", 0, i)));
    }

    #[test]
    fn rejects_a_paused_caller_without_spending_a_token() {
        let (policy, limiter) = (policy(), RateLimiter::default());
        let alice = default_caller("alice", "", 0);
        let input = PolicyInput {
            policy: &policy,
            caller: &alice,
            capability: None,
            used_this_month: 0,
            caller_paused: true,
            caller_name: "Alice's Nandi",
        };
        match evaluate(input, &limiter, 0) {
            PolicyDecision::Reject {
                reason,
                kind,
                limit,
            } => {
                assert_eq!(reason, "Requests from Alice's Nandi are paused");
                assert_eq!(kind, ReasonKind::PausedOrBlocked);
                assert_eq!(limit, None);
            }
            other => panic!("expected a rejection, got {other:?}"),
        }
        // Both tokens are still there
        assert!(limiter.try_take("alice", 2, 0));
        assert!(limiter.try_take("alice", 2, 0));
    }

    #[test]
    fn enforces_the_monthly_budget() {
        let (policy, limiter) = (policy(), RateLimiter::default());
        let caller = CallerPolicy {
            monthly_task_budget: Some(5),
            ..default_caller("alice", "", 0)
        };
        assert!(allowed(&check(&policy, &caller, None, 4, &limiter, 0)));
        assert!(matches!(
            check(&policy, &caller, None, 5, &limiter, 0),
            PolicyDecision::Reject {
                limit: Some(LimitKind::MonthlyBudget),
                ..
            }
        ));
    }

    #[test]
    fn applies_exposure_and_caller_capabilities() {
        let limiter = RateLimiter::default();
        let policy = GatewayPolicy {
            expose: vec!["summarize".into(), "translate".into()],
            hide: vec!["translate".into()],
            require_approval: vec!["summarize".into()],
            rate_limit_per_hour: 0,
            ..GatewayPolicy::default()
        };
        let caller = default_caller("alice", "", 0);
        assert_eq!(
            check(&policy, &caller, Some("Summarize"), 0, &limiter, 0),
            PolicyDecision::Allow {
                needs_approval: true
            }
        );
        assert!(!allowed(&check(
            &policy,
            &caller,
            Some("translate"),
            0,
            &limiter,
            0
        )));
        assert!(!allowed(&check(
            &policy,
            &caller,
            Some("write"),
            0,
            &limiter,
            0
        )));
        // Natural-language requests: the router decides
        assert_eq!(
            check(&policy, &caller, None, 0, &limiter, 0),
            PolicyDecision::Allow {
                needs_approval: false
            }
        );
        let narrow = CallerPolicy {
            allowed_capabilities: vec!["translate".into()],
            ..caller.clone()
        };
        assert!(!allowed(&check(
            &policy,
            &narrow,
            Some("summarize"),
            0,
            &limiter,
            0
        )));
        assert!(is_exposed(&GatewayPolicy::default(), "anything"));
    }

    #[test]
    fn every_rejection_has_a_kind_and_plain_words() {
        let limiter = RateLimiter::default();
        let policy = GatewayPolicy {
            expose: vec!["summarize".into()],
            rate_limit_per_hour: 1,
            ..GatewayPolicy::default()
        };
        let caller = default_caller("desktop-xyz", "", 0);
        let kind_of = |decision: PolicyDecision| match decision {
            PolicyDecision::Reject { reason, kind, .. } => {
                assert!(!reason.contains("desktop-xyz"), "a device ID: {reason}");
                kind
            }
            other => panic!("expected a rejection, got {other:?}"),
        };
        let disabled = CallerPolicy {
            disabled: true,
            ..caller.clone()
        };
        assert_eq!(
            kind_of(check(&policy, &disabled, None, 0, &limiter, 0)),
            ReasonKind::PausedOrBlocked
        );
        assert_eq!(
            kind_of(check(&policy, &caller, Some("write"), 0, &limiter, 0)),
            ReasonKind::NotAccepting
        );
        let narrow = CallerPolicy {
            allowed_capabilities: vec!["translate".into()],
            ..caller.clone()
        };
        assert_eq!(
            kind_of(check(&policy, &narrow, Some("summarize"), 0, &limiter, 0)),
            ReasonKind::NotAccepting
        );
        let budgeted = CallerPolicy {
            monthly_task_budget: Some(1),
            ..caller.clone()
        };
        assert_eq!(
            kind_of(check(&policy, &budgeted, None, 1, &limiter, 0)),
            ReasonKind::Failed
        );
        assert!(allowed(&check(&policy, &caller, None, 0, &limiter, 0)));
        assert_eq!(
            kind_of(check(&policy, &caller, None, 0, &limiter, 1)),
            ReasonKind::Failed
        );
    }

    #[test]
    fn a_disabled_caller_is_refused_without_spending_a_token() {
        let (policy, limiter) = (policy(), RateLimiter::default());
        let caller = CallerPolicy {
            disabled: true,
            ..default_caller("alice", "", 0)
        };
        assert!(matches!(
            check(&policy, &caller, None, 0, &limiter, 0),
            PolicyDecision::Reject { limit: None, .. }
        ));
        assert!(limiter.try_take("alice", 2, 0));
        assert!(limiter.try_take("alice", 2, 0));
    }

    #[test]
    fn callers_and_usage_round_trip() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        let mut caller = CallerPolicy {
            allowed_capabilities: vec!["summarize".into()],
            monthly_task_budget: Some(10),
            ..default_caller("alice-laptop", "alice", 1_000)
        };
        upsert_caller(&conn, &caller).unwrap();
        caller.disabled = true;
        caller.created_at = 5_000.0; // kept from the first insert
        upsert_caller(&conn, &caller).unwrap();
        let listed = list_callers(&conn).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].disabled);
        assert_eq!(listed[0].created_at, 1_000.0);
        assert_eq!(listed[0].allowed_capabilities, ["summarize"]);

        let month = month_of(1_760_000_000_000);
        assert_eq!(month, "2025-10");
        increment_usage(&conn, "alice-laptop", &month).unwrap();
        increment_usage(&conn, "alice-laptop", &month).unwrap();
        let used = usage(&conn, "alice-laptop", &month).unwrap();
        assert_eq!(used.task_count, 2);
        assert_eq!(used.monthly_task_budget, Some(10));
        assert_eq!(usage_count(&conn, "alice-laptop", "2025-11").unwrap(), 0);

        assert!(remove_caller(&conn, "alice-laptop").unwrap());
        assert!(!remove_caller(&conn, "alice-laptop").unwrap());
        assert!(get_caller(&conn, "alice-laptop").unwrap().is_none());
    }
}
