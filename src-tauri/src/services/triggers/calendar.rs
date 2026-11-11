//! Calendar source: one task per enabled calendar trigger that sleeps until
//! the next scheduled time (local time) and then acts on it as this
//! Nandi's role that day says (rules in `domain::calendar`):
//!
//! | Role                                        | At the scheduled time                              |
//! | ------------------------------------------- | -------------------------------------------------- |
//! | **Owner**: not named in the calendar        | Fires for the primary; watches the run, and fires  |
//! |                                             | again for the backup if it fails or runs late      |
//! | **Primary**: named, and it's their day      | Fires; the backup's Nandi watches                 |
//! | **Backup**: named as today's backup         | Waits `GRACE`, then asks the primary's Nandi and  |
//! |                                             | fires for itself if it's offline or didn't do it   |
//! | **Bystander**: named, but not today         | Nothing                                            |
//!
//! So the same trigger can sit in everyone's repo, and whoever is on duty
//! runs it. Each fire's payload carries `duty_id`, `role`, `responsible`,
//! `backup`, and (on a handover) `primary` and `handover_reason`.

use std::sync::{Arc, OnceLock};

use chrono::{DateTime, Local, NaiveDateTime, TimeZone};
use serde::Serialize;
use serde_json::{json, Value};
use specta::Type;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use super::handover::{self, DutyNetwork, Watch};
use super::schema::{TriggerEvent, TriggerSourceType};
use crate::db::Db;
use crate::model::Timestamp;
use crate::services::domain::calendar::{is_one_of, CalendarFire, CalendarSpec, Duty, DutyId};

pub fn parse(config: &Value) -> Result<CalendarSpec, String> {
    CalendarSpec::from_value(config.clone()).map_err(|e| e.to_string())
}

/// A wall-clock time on this machine (the earlier one when clocks go back;
/// `None` for a time skipped when they go forward).
pub fn local(at: NaiveDateTime) -> Option<DateTime<Local>> {
    Local.from_local_datetime(&at).earliest()
}

/// The next fire strictly after `after` that exists on this machine's clock.
pub fn next_fire(
    spec: &CalendarSpec,
    after: DateTime<Local>,
) -> Option<(CalendarFire, DateTime<Local>)> {
    let mut cursor = after.naive_local();
    // A few skipped (daylight-saving) times at most
    for _ in 0..8 {
        let fire = spec.next_after(cursor)?;
        if let Some(at) = local(fire.at).filter(|at| *at > after) {
            return Some((fire, at));
        }
        cursor = fire.at;
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Owner,
    Primary,
    Backup,
    Bystander,
}

/// This Nandi's part in a fire, given the names it answers to.
pub fn role(spec: &CalendarSpec, duty: Option<&Duty>, me: &[String]) -> Role {
    if me.is_empty() || !spec.mentions(me) {
        return Role::Owner;
    }
    match duty {
        Some(duty) if is_one_of(&duty.person, me) => Role::Primary,
        Some(duty) if duty.backup.as_deref().is_some_and(|b| is_one_of(b, me)) => Role::Backup,
        _ => Role::Bystander,
    }
}

/// The primary's fire.
pub fn payload(duty_id: &DutyId, scheduled_at: DateTime<Local>, duty: Option<&Duty>) -> Value {
    json!({
        "scheduled_at": scheduled_at.to_rfc3339(),
        "trigger_id": duty_id.trigger_id,
        "duty_id": duty_id.to_string(),
        "date": duty_id.at.date().to_string(),
        "role": "primary",
        "responsible": duty.map(|d| &d.person),
        "backup": duty.and_then(|d| d.backup.as_ref()),
        "covering_for": duty.and_then(|d| d.covering_for.as_ref()),
        "duty_reason": duty.map(|d| d.reason),
    })
}

/// The backup's fire, made from the primary's: the backup is now
/// responsible, and `reason` says why.
pub fn handover_payload(primary: &Value, reason: &str) -> Value {
    let mut payload = primary.clone();
    if let Some(map) = payload.as_object_mut() {
        let person = map.get("responsible").cloned().unwrap_or(Value::Null);
        let backup = map.get("backup").cloned().unwrap_or(Value::Null);
        map.insert("role".into(), json!("backup"));
        map.insert("primary".into(), person);
        map.insert("responsible".into(), backup);
        map.insert("backup".into(), Value::Null);
        map.insert("handover_reason".into(), json!(reason));
    }
    payload
}

/// Next occurrences a preview lists
pub const PREVIEW_COUNT: usize = 5;

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CalendarRun {
    /// Epoch ms, local time zone
    pub at: Timestamp,
    pub person: Option<String>,
    pub backup: Option<String>,
    pub covering_for: Option<String>,
}

/// A calendar checked as the user edits it: whether it's valid and its
/// next runs with who is on duty. Uses the scheduler's own rules.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct CalendarPreview {
    pub valid: bool,
    pub error: Option<String>,
    pub next: Vec<CalendarRun>,
}

pub fn preview(config: &Value, after: DateTime<Local>, count: usize) -> CalendarPreview {
    let spec = match parse(config) {
        Ok(spec) => spec,
        Err(error) => {
            return CalendarPreview {
                valid: false,
                error: Some(error),
                next: Vec::new(),
            }
        }
    };
    let mut next = Vec::with_capacity(count);
    let mut cursor = after;
    while next.len() < count {
        let Some((fire, at)) = next_fire(&spec, cursor) else {
            break;
        };
        let duty = fire.duty;
        next.push(CalendarRun {
            at: at.timestamp_millis() as Timestamp,
            person: duty.as_ref().map(|d| d.person.clone()),
            backup: duty.as_ref().and_then(|d| d.backup.clone()),
            covering_for: duty.and_then(|d| d.covering_for),
        });
        cursor = at;
    }
    CalendarPreview {
        valid: true,
        error: None,
        next,
    }
}

/// What a calendar task needs besides its rule.
#[derive(Clone)]
pub struct CalendarDeps {
    pub runtime: tokio::runtime::Handle,
    pub events: mpsc::Sender<TriggerEvent>,
    pub db: Db,
    /// Set once the relay is up; read at each fire
    pub network: Arc<OnceLock<Arc<dyn DutyNetwork>>>,
}

/// Starts the job; abort the handle to stop it (on reload or disable).
/// Handovers it started keep going.
pub fn spawn(deps: CalendarDeps, trigger_id: String, spec: CalendarSpec) -> AbortHandle {
    let runtime = deps.runtime.clone();
    runtime
        .spawn(async move {
            let mut after = Local::now();
            loop {
                let Some((fire, at)) = next_fire(&spec, after) else {
                    log::warn!("Calendar trigger '{trigger_id}' has no fires in the next year");
                    return;
                };
                let wait = (at - Local::now()).to_std().unwrap_or_default();
                tokio::time::sleep(wait).await;
                if !act(&deps, &trigger_id, &spec, fire, at).await {
                    return; // engine stopped
                }
                // Never act on the same time twice, even if the sleep woke
                // a little early
                after = at.max(Local::now());
            }
        })
        .abort_handle()
}

/// Does this Nandi's part in one fire. `false` once the engine is gone.
async fn act(
    deps: &CalendarDeps,
    trigger_id: &str,
    spec: &CalendarSpec,
    fire: CalendarFire,
    at: DateTime<Local>,
) -> bool {
    let network = deps.network.get().cloned();
    let me = network.as_ref().map(|n| n.my_names()).unwrap_or_default();
    let duty_id = DutyId {
        trigger_id: trigger_id.to_string(),
        at: fire.at,
    };
    let duty = fire.duty.as_ref();
    let primary = payload(&duty_id, at, duty);
    let backup = duty.and_then(|d| d.backup.clone());
    let watch = match role(spec, duty, &me) {
        Role::Bystander => {
            log::debug!("Calendar trigger '{trigger_id}': not on duty at {duty_id}");
            return true;
        }
        Role::Owner | Role::Primary => {
            let event = TriggerEvent {
                trigger_id: trigger_id.to_string(),
                source: TriggerSourceType::Calendar,
                payload: primary.clone(),
                test: false,
                handover: false,
            };
            if deps.events.send(event).await.is_err() {
                return false;
            }
            // A primary's backup watches from their own Nandi
            (role(spec, duty, &me) == Role::Owner && backup.is_some()).then(|| Watch::Local {
                db: deps.db.clone(),
            })
        }
        Role::Backup => match (network, duty) {
            (Some(network), Some(duty)) => Some(Watch::Remote {
                network,
                primary: duty.person.clone(),
            }),
            _ => None,
        },
    };
    if let Some(watch) = watch {
        handover::spawn(
            &deps.runtime,
            watch,
            duty_id,
            spec.backup_after_minutes(),
            primary,
            deps.events.clone(),
        );
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    fn spec(config: Value) -> CalendarSpec {
        parse(&config).unwrap()
    }

    #[test]
    fn previews_who_is_on_duty() {
        let config = json!({
            "times": ["09:00"], "days": ["mon", "tue", "wed", "thu", "fri"],
            "people": ["alice", "bob"], "rotate": "day", "startDate": "2026-10-26"
        });
        let friday = Local.with_ymd_and_hms(2026, 10, 30, 10, 0, 0).unwrap();
        let p = preview(&config, friday, 3);
        assert!(p.valid);
        let days: Vec<_> = p
            .next
            .iter()
            .map(|run| {
                let at = Local.timestamp_millis_opt(run.at as i64).unwrap();
                (
                    at.day(),
                    at.hour(),
                    run.person.clone().unwrap(),
                    run.backup.clone().unwrap(),
                )
            })
            .collect();
        // Mon 2nd is bob's (Mon a, Tue b, Wed a, Thu b, Fri a, Mon b)
        assert_eq!(
            days,
            [
                (2, 9, "bob".into(), "alice".into()),
                (3, 9, "alice".into(), "bob".into()),
                (4, 9, "bob".into(), "alice".into()),
            ]
        );
        let bad = preview(&json!({"times": []}), friday, 3);
        assert!(!bad.valid && bad.error.unwrap().contains("at least one time"));
    }

    #[test]
    fn roles_follow_the_names_this_nandi_answers_to() {
        let s = spec(json!({
            "times": ["09:00"], "people": ["alice", "bob", "carol"], "startDate": "2026-10-26"
        }));
        let duty = s
            .duty_on(chrono::NaiveDate::from_ymd_opt(2026, 10, 27).unwrap())
            .unwrap();
        let names = |n: &[&str]| n.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            role(&s, Some(&duty), &names(&["Alice", "alice-pc-1"])),
            Role::Primary
        );
        assert_eq!(role(&s, Some(&duty), &names(&["bob"])), Role::Backup);
        assert_eq!(role(&s, Some(&duty), &names(&["carol"])), Role::Bystander);
        // A team lead's Nandi (not in the rota), or one without a relay
        assert_eq!(role(&s, Some(&duty), &names(&["lead"])), Role::Owner);
        assert_eq!(role(&s, Some(&duty), &[]), Role::Owner);
    }

    #[test]
    fn handovers_make_the_backup_responsible() {
        let s = spec(json!({"times": ["09:00"], "byDay": {"tue": "alice"}, "backup": "bob"}));
        let fire = s
            .next_after(
                chrono::NaiveDate::from_ymd_opt(2026, 10, 26)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
            )
            .unwrap();
        let id = DutyId {
            trigger_id: "standup".into(),
            at: fire.at,
        };
        let primary = payload(&id, local(fire.at).unwrap(), fire.duty.as_ref());
        assert_eq!(primary["duty_id"], "standup/2026-10-27/09:00");
        assert_eq!(primary["responsible"], "alice");
        assert_eq!(primary["backup"], "bob");
        assert_eq!(primary["duty_reason"], "byDay");

        let backup = handover_payload(&primary, "the run failed");
        assert_eq!(backup["role"], "backup");
        assert_eq!(backup["responsible"], "bob");
        assert_eq!(backup["primary"], "alice");
        assert_eq!(backup["backup"], Value::Null);
        assert_eq!(backup["handover_reason"], "the run failed");
        assert_eq!(backup["duty_id"], primary["duty_id"]);
    }
}
