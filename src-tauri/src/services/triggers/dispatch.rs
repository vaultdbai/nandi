//! Turns a fire into work. For each `TriggerEvent`, in order:
//!
//! 1. `rate_limit_seconds`: too soon after the last dispatched fire records
//!    a `rate_limited` fire and emits `trigger://rate_limited`.
//! 2. Storm guard: more than `STORM_MAX_FIRES` in `STORM_WINDOW_MS` of the
//!    fires left (cron and test fires excepted) disables the trigger and
//!    emits `trigger://error`.
//! 3. An event trigger ignores runs it dispatched itself (no self-loops).
//! 4. Render `input_template`, write the `trigger_fires` row (DB first),
//!    then enqueue an agent run (or start a workflow run) and record the
//!    run on the fire. Emits `trigger://fired` or `trigger://error`.
//!
//! Fires are processed one at a time (see `TriggerEngine::dispatching`), so
//! rate-limit checks never race.

use std::collections::{HashMap, VecDeque};

use serde_json::{json, Value};

use super::schema::{
    Trigger, TriggerEvent, TriggerFire, TriggerFireStatus, TriggerSourceType, TriggerTargetType,
    STORM_MAX_FIRES, STORM_WINDOW_MS,
};
use super::template::{self, TemplateContext};
use super::{store, TriggerEngine, TriggerError, TriggerFired, TriggerRateLimited};
use crate::audit::{self, AuditLog};
use crate::db;
use crate::services::chat;
use crate::services::events;
use crate::services::orchestrator::RunSource;
use crate::services::workflows::WorkflowRunSource;

/// Recent fire times per trigger.
#[derive(Default)]
pub struct StormDetector {
    recent: HashMap<String, VecDeque<i64>>,
}

impl StormDetector {
    /// Records a fire at `now`; true if it makes a storm.
    pub fn record(&mut self, trigger_id: &str, now: i64) -> bool {
        let times = self.recent.entry(trigger_id.to_string()).or_default();
        while times.front().is_some_and(|t| now - t >= STORM_WINDOW_MS) {
            times.pop_front();
        }
        times.push_back(now);
        times.len() > STORM_MAX_FIRES
    }

    pub fn forget(&mut self, trigger_id: &str) {
        self.recent.remove(trigger_id);
    }
}

/// When a rate-limited trigger may fire again, if `now` is too soon.
pub fn rate_limited_until(trigger: &Trigger, now: i64) -> Option<i64> {
    let limit = i64::from(trigger.rate_limit_seconds?) * 1000;
    let next = trigger.last_fired_at? as i64 + limit;
    (limit > 0 && now < next).then_some(next)
}

fn iso(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339()
}

/// Processes one fire. Returns the recorded fire, or `None` if the trigger
/// is gone or (outside a test) disabled or of another source type.
pub async fn process(engine: &TriggerEngine, event: TriggerEvent) -> Option<TriggerFire> {
    let trigger = engine
        .trigger(&event.trigger_id)
        .filter(|t| event.test || (t.enabled && t.source_type == event.source))?;
    let now = db::now_ms();
    let mut fire = TriggerFire {
        id: uuid::Uuid::new_v4().to_string(),
        trigger_id: trigger.id.clone(),
        fired_at: db::ts(now),
        payload: event.payload.clone(),
        run_id: None,
        status: TriggerFireStatus::Queued,
        error: None,
        run_status: None,
    };

    // A backup taking over is the same duty, not another fire
    let handover = event.payload.get("role").and_then(Value::as_str) == Some("backup");
    if !event.test && !handover {
        if let Some(next) = rate_limited_until(&trigger, now) {
            let error = format!("Rate limited until {}", iso(next));
            let fire = record(engine, fire, TriggerFireStatus::RateLimited, Some(error)).await?;
            events::emit(
                &engine.bus,
                &TriggerRateLimited {
                    trigger_id: trigger.id.clone(),
                    fire_id: fire.id.clone(),
                    next_allowed_at: db::ts(next),
                },
            );
            return Some(fire);
        }
    }

    // Only fires that would run count: a rate-limited trigger's burst (a
    // folder of files copied at once) is one run, not a storm
    let storm = !event.test
        && !matches!(
            trigger.source_type,
            TriggerSourceType::Cron | TriggerSourceType::Calendar
        )
        && engine.record_fire(&trigger.id, now);
    if storm {
        let error = format!(
            "Fired more than {STORM_MAX_FIRES} times in {}s, so it was disabled",
            STORM_WINDOW_MS / 1000
        );
        log::warn!("Trigger '{}': {error}", trigger.id);
        let fire = record(engine, fire, TriggerFireStatus::Error, Some(error.clone())).await?;
        events::emit(
            &engine.bus,
            &TriggerError {
                trigger_id: trigger.id.clone(),
                fire_id: Some(fire.id.clone()),
                error,
            },
        );
        if let Err(e) = engine.set_enabled(&trigger.id, false).await {
            log::error!("Failed to disable trigger '{}': {e}", trigger.id);
        }
        return Some(fire);
    }

    if trigger.source_type == TriggerSourceType::Event
        && event.payload.get("source_id").and_then(Value::as_str) == Some(trigger.id.as_str())
    {
        let reason = "Ignored: the run was started by this trigger".to_string();
        return record(engine, fire, TriggerFireStatus::Skipped, Some(reason)).await;
    }

    // Render the input before writing, so the fire row is complete
    let (trigger_id, uses_last) = (
        trigger.id.clone(),
        trigger
            .input_template
            .as_deref()
            .is_some_and(|t| t.contains("last_run.output")),
    );
    let last_output = if uses_last {
        engine
            .db
            .call(move |conn| store::last_output(conn, &trigger_id))
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    let input = match &trigger.input_template {
        Some(template) => {
            let fired_at = iso(now);
            let (text, missing) = template::render(
                template,
                &TemplateContext {
                    payload: &event.payload,
                    trigger_id: &trigger.id,
                    trigger_name: &trigger.name,
                    fired_at: &fired_at,
                    last_output: last_output.as_deref(),
                },
            );
            if !missing.is_empty() {
                log::warn!(
                    "Trigger '{}' template has no value for: {}",
                    trigger.id,
                    missing.join(", ")
                );
            }
            text
        }
        None => template::default_input(&trigger.name, &event.payload),
    };

    // DB first: the fire is recorded before anything is dispatched
    let row = fire.clone();
    if let Err(e) = engine
        .db
        .call(move |conn| store::insert_fire(conn, &row))
        .await
    {
        log::error!("Failed to record a fire of '{}': {e}", trigger.id);
        return None;
    }

    let dispatched = dispatch(engine, &trigger, input).await;
    let (status, run_id, error) = match &dispatched {
        Ok(run_id) => (TriggerFireStatus::Queued, Some(run_id.clone()), None),
        Err(e) => (TriggerFireStatus::Error, None, Some(e.clone())),
    };
    let (fire_id, id, at) = (fire.id.clone(), trigger.id.clone(), now);
    let (run, err) = (run_id.clone(), error.clone());
    let updated = engine
        .db
        .call(move |conn| {
            if status == TriggerFireStatus::Queued {
                store::set_last_fired(conn, &id, at)?;
            }
            store::finish_fire(conn, &fire_id, status, run.as_deref(), err.as_deref())
        })
        .await;
    fire.status = status;
    fire.run_id = run_id;
    fire.error = error;
    let fire = match updated {
        Ok(Some(saved)) => saved,
        Ok(None) => fire,
        Err(e) => {
            log::error!("Failed to update a fire of '{}': {e}", trigger.id);
            fire
        }
    };

    match &dispatched {
        Ok(_) => {
            engine.note_fired(&trigger.id, now);
            events::emit(
                &engine.bus,
                &TriggerFired {
                    trigger_id: trigger.id.clone(),
                    fire_id: fire.id.clone(),
                    source_type: trigger.source_type,
                    payload: event.payload.clone(),
                    run_id: fire.run_id.clone(),
                },
            );
        }
        Err(e) => {
            log::warn!("Trigger '{}' couldn't dispatch: {e}", trigger.id);
            events::emit(
                &engine.bus,
                &TriggerError {
                    trigger_id: trigger.id.clone(),
                    fire_id: Some(fire.id.clone()),
                    error: e.clone(),
                },
            );
        }
    }
    audit_fire(&trigger, &fire).await;
    Some(fire)
}

/// Enqueues the run; returns its ID (the workflow run's, for workflows).
async fn dispatch(
    engine: &TriggerEngine,
    trigger: &Trigger,
    input: String,
) -> Result<String, String> {
    match trigger.target_type {
        TriggerTargetType::Agent => {
            // Tests have no agent folders
            if !cfg!(test) {
                engine.store.require_agent(&trigger.target_id)?;
            }
            let agent_id = trigger.target_id.clone();
            let conversation = engine
                .db
                .call(move |conn| db::ensure_agent_conversation(conn, &agent_id))
                .await?;
            let source = RunSource {
                source_type: trigger.source_type.run_source(),
                source_id: Some(trigger.id.clone()),
                source_label: Some(trigger.name.clone()),
                priority: trigger.priority,
            };
            // Queued behind the agent's current turn (whether or not its
            // container is running) by the orchestrator
            chat::submit(
                &engine.orchestrators,
                &trigger.target_id,
                conversation,
                input,
                vec![],
                None,
                source,
            )
            .await
            .map(|sent| sent.run_id)
        }
        // Workflows take no input yet: their steps' templates apply
        TriggerTargetType::Workflow => {
            engine
                .workflows
                .run_workflow_from(
                    &trigger.target_id,
                    WorkflowRunSource::Trigger(trigger.id.clone()),
                )
                .await
        }
    }
}

/// Writes a fire that dispatches nothing, and audits it.
async fn record(
    engine: &TriggerEngine,
    mut fire: TriggerFire,
    status: TriggerFireStatus,
    error: Option<String>,
) -> Option<TriggerFire> {
    fire.status = status;
    fire.error = error;
    let row = fire.clone();
    if let Err(e) = engine
        .db
        .call(move |conn| store::insert_fire(conn, &row))
        .await
    {
        log::error!("Failed to record a fire of '{}': {e}", fire.trigger_id);
        return None;
    }
    if let Some(trigger) = engine.trigger(&fire.trigger_id) {
        audit_fire(&trigger, &fire).await;
    }
    Some(fire)
}

/// Metadata only: never the payload (it may hold anything a sender posted).
async fn audit_fire(trigger: &Trigger, fire: &TriggerFire) {
    audit::record(
        AuditLog::Triggers,
        &json!({
            "ts": audit::now_ms(),
            "trigger_id": trigger.id,
            "fire_id": fire.id,
            "source_type": trigger.source_type.as_str(),
            "target_type": trigger.target_type.as_str(),
            "target_id": trigger.target_id,
            "status": fire.status.as_str(),
            "run_id": fire.run_id,
            "error": fire.error,
        }),
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::triggers::schema::TriggerTargetType;

    #[test]
    fn storms_are_more_than_five_fires_in_ten_seconds() {
        let mut storms = StormDetector::default();
        for i in 0..5 {
            assert!(!storms.record("t", i * 1000), "fire {i} is fine");
        }
        assert!(storms.record("t", 5_000), "the 6th within 10s is a storm");
        assert!(!storms.record("other", 5_000));

        // Spread out, the same count is fine
        let mut calm = StormDetector::default();
        for i in 0..20 {
            assert!(!calm.record("t", i * 2_500));
        }
    }

    #[test]
    fn rate_limits_relative_to_the_last_dispatched_fire() {
        let mut trigger = Trigger {
            id: "t".into(),
            name: "T".into(),
            enabled: true,
            source_type: TriggerSourceType::Webhook,
            source_config: json!({}),
            target_type: TriggerTargetType::Agent,
            target_id: "a".into(),
            input_template: None,
            priority: 60,
            rate_limit_seconds: Some(60),
            created_at: 0.0,
            updated_at: 0.0,
            last_fired_at: None,
            has_secret: false,
        };
        assert_eq!(rate_limited_until(&trigger, 1_000), None, "never fired");
        trigger.last_fired_at = Some(10_000.0);
        assert_eq!(rate_limited_until(&trigger, 30_000), Some(70_000));
        assert_eq!(rate_limited_until(&trigger, 70_000), None);
        trigger.rate_limit_seconds = None;
        assert_eq!(rate_limited_until(&trigger, 10_001), None);
        trigger.rate_limit_seconds = Some(0);
        assert_eq!(rate_limited_until(&trigger, 10_001), None);
    }
}
