//! Trigger engine integration tests: real sources, dispatcher, and agent
//! queue over an in-memory core (canned engine, no agent folders).

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::schema::{TriggerFireStatus, TriggerTargetType};
use super::*;
use crate::model::{RunStatus, SourceType};
use crate::services::chat;
use crate::services::events::tests::RecordingSink;
use crate::services::orchestrator::RunSource;

async fn app() -> (AppState, Arc<RecordingSink>) {
    let recorder = Arc::new(RecordingSink::default());
    let state = AppState::for_tests(vec![recorder.clone()]);
    state
        .db()
        .call(|conn| {
            db::set_setting(conn, db::ENGINE_KEY, &json!("canned"))?;
            let mut script = crate::engine::default_script();
            script["tokenDelayMs"] = json!(0);
            db::set_setting(conn, crate::engine::CANNED_SCRIPT_KEY, &script)
        })
        .await
        .unwrap();
    state.triggers().start().await.unwrap();
    (state, recorder)
}

fn input(name: &str, source_type: TriggerSourceType, config: Value) -> TriggerInput {
    TriggerInput {
        id: None,
        name: name.into(),
        enabled: None,
        source_type,
        source_config: config,
        target_type: TriggerTargetType::Agent,
        target_id: "writer".into(),
        input_template: None,
        priority: None,
        rate_limit_seconds: None,
    }
}

async fn fires(state: &AppState, id: &str) -> Vec<TriggerFire> {
    list_trigger_fires(state, id.into(), Some(500))
        .await
        .unwrap()
}

/// Waits until the trigger has `count` settled fires (oldest first). A
/// fire is written before it dispatches, so a `queued` one without a run
/// is still in flight.
async fn wait_for_fires(state: &AppState, id: &str, count: usize) -> Vec<TriggerFire> {
    for _ in 0..500 {
        let mut list = fires(state, id).await;
        let settled = list
            .iter()
            .all(|f| f.status != TriggerFireStatus::Queued || f.run_id.is_some());
        if list.len() >= count && settled {
            list.reverse();
            return list;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("trigger '{id}' never reached {count} fires");
}

async fn run(state: &AppState, run_id: &str) -> crate::model::RunSummary {
    let id = run_id.to_string();
    state
        .db()
        .call(move |conn| db::get_run(conn, &id))
        .await
        .unwrap()
        .unwrap()
}

fn count(recorder: &RecordingSink, name: &str) -> usize {
    recorder.names().iter().filter(|n| **n == name).count()
}

#[tokio::test]
async fn a_webhook_queues_a_run_with_its_source_and_payload() {
    let (state, recorder) = app().await;
    let mut hook = input("Orders", TriggerSourceType::Webhook, json!({}));
    hook.input_template = Some("Order {{payload.order.id}} via {{trigger.name}}".into());
    let saved = create_trigger(&state, hook).await.unwrap();
    let secret = saved.secret.clone().expect("the secret is shown once");
    assert!(saved.trigger.has_secret);
    assert_eq!(saved.trigger.source_config, json!({}));
    assert!(
        !serde_json::to_string(&list_triggers(&state).await.unwrap())
            .unwrap()
            .contains(&secret)
    );

    let engine = state.triggers();
    let id = saved.trigger.id.as_str();
    assert_eq!(
        engine.receive_webhook(id, Some("wrong"), json!({})),
        Err(ReceiveError::Unauthorized)
    );
    assert_eq!(
        engine.receive_webhook(id, None, json!({})),
        Err(ReceiveError::Unauthorized)
    );
    assert_eq!(
        engine.receive_webhook("missing", Some(&secret), json!({})),
        Err(ReceiveError::NotFound)
    );
    let payload = json!({"order": {"id": 42}});
    assert_eq!(
        engine.receive_webhook(id, Some(&secret), payload.clone()),
        Ok(())
    );

    let [fire] = &wait_for_fires(&state, id, 1).await[..] else {
        unreachable!()
    };
    assert_eq!(fire.status, TriggerFireStatus::Queued, "{:?}", fire.error);
    assert_eq!(fire.payload, payload);
    let run = run(&state, fire.run_id.as_deref().unwrap()).await;
    assert_eq!(run.agent_id, "writer");
    assert_eq!(run.source_type, SourceType::Webhook);
    assert_eq!(run.source_id.as_deref(), Some(id));
    assert_eq!(run.source_label.as_deref(), Some("Orders"));
    assert_eq!(run.priority, 60);
    assert_eq!(run.input, "Order 42 via Orders");
    // The fire carries its run's status, and follows it to the end
    assert!(fire.run_status.is_some(), "{fire:?}");
    let run_id = fire.run_id.clone().unwrap();
    state
        .db()
        .call(move |conn| {
            conn.execute(
                "UPDATE runs SET status = 'completed' WHERE id = ?1",
                [&run_id],
            )
        })
        .await
        .unwrap();
    let [settled] = &fires(&state, id).await[..] else {
        unreachable!()
    };
    assert_eq!(settled.status, TriggerFireStatus::Queued);
    assert_eq!(
        settled.run_status,
        Some(crate::services::triggers::schema::FireRunStatus::Completed)
    );
    assert_eq!(count(&recorder, "trigger://fired"), 1);
    let trigger = get_trigger(&state, id.into()).await.unwrap();
    assert_eq!(trigger.last_fired_at, Some(fire.fired_at));

    // Disabled triggers don't exist as far as callers can tell
    set_trigger_enabled(&state, id.into(), false).await.unwrap();
    assert_eq!(
        engine.receive_webhook(id, Some(&secret), json!({})),
        Err(ReceiveError::NotFound)
    );
    state.shutdown();
}

#[tokio::test]
async fn a_fast_cron_dispatches_exactly_one_run_per_fire() {
    let (state, _recorder) = app().await;
    let saved = create_trigger(
        &state,
        input(
            "Tick",
            TriggerSourceType::Cron,
            json!({"schedule": "* * * * * *"}),
        ),
    )
    .await
    .unwrap();
    let id = saved.trigger.id.clone();
    assert!(state.triggers().is_watching(&id));

    let list = wait_for_fires(&state, &id, 2).await;
    set_trigger_enabled(&state, id.clone(), false)
        .await
        .unwrap();
    assert!(!state.triggers().is_watching(&id));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let list = {
        let mut all = fires(&state, &id).await;
        all.reverse();
        assert!(all.len() >= list.len());
        all
    };

    let mut scheduled: Vec<&str> = list
        .iter()
        .map(|f| f.payload["scheduled_at"].as_str().unwrap())
        .collect();
    scheduled.dedup();
    assert_eq!(scheduled.len(), list.len(), "no occurrence fired twice");
    let run_ids: Vec<String> = list.iter().filter_map(|f| f.run_id.clone()).collect();
    assert_eq!(run_ids.len(), list.len(), "every fire dispatched one run");
    let trigger_id = id.clone();
    let runs = state
        .db()
        .call(move |conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM runs WHERE source_id = ?1 AND source_type = 'cron'",
                [trigger_id],
                |row| row.get::<_, i64>(0),
            )
        })
        .await
        .unwrap();
    assert_eq!(runs as usize, list.len());
    // Cron is exempt from the storm guard, so it's still just disabled by us
    assert!(list.iter().all(|f| f.status == TriggerFireStatus::Queued));
    state.shutdown();
}

#[tokio::test]
async fn the_file_watcher_fires_once_per_debounced_burst() {
    let (state, _recorder) = app().await;
    let dir = tempfile::tempdir().unwrap();
    let mut watch = input(
        "Contracts",
        TriggerSourceType::File,
        json!({
            "path": dir.path().to_string_lossy(),
            "patterns": ["*.pdf"],
            "debounceMs": file::TEST_DEBOUNCE.as_millis() as u64,
        }),
    );
    watch.input_template = Some("Review {{payload.path}} ({{payload.event}})".into());
    let id = create_trigger(&state, watch).await.unwrap().trigger.id;
    tokio::time::sleep(Duration::from_millis(100)).await;

    let pdf = dir.path().join("deal.pdf");
    std::fs::write(&pdf, "v1").unwrap();
    for i in 0..5 {
        std::fs::write(&pdf, format!("v{i} and more")).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();

    let list = wait_for_fires(&state, &id, 1).await;
    tokio::time::sleep(file::TEST_DEBOUNCE * 3).await;
    assert_eq!(fires(&state, &id).await.len(), 1, "one fire for the burst");
    let fire = &list[0];
    assert_eq!(fire.status, TriggerFireStatus::Queued, "{:?}", fire.error);
    assert!(fire.payload["path"].as_str().unwrap().ends_with("deal.pdf"));
    assert_eq!(fire.payload["event"], "create");
    let run = run(&state, fire.run_id.as_deref().unwrap()).await;
    assert_eq!(run.source_type, SourceType::File);
    assert_eq!(run.priority, 70);
    assert!(run.input.starts_with("Review ") && run.input.ends_with("deal.pdf (create)"));
    state.shutdown();
}

#[tokio::test]
async fn file_triggers_need_an_existing_folder() {
    let (state, _recorder) = app().await;
    let error = create_trigger(
        &state,
        input(
            "Gone",
            TriggerSourceType::File,
            json!({"path": "/no/such/folder"}),
        ),
    )
    .await
    .unwrap_err();
    assert!(error.contains("not a folder"), "{error}");
    state.shutdown();
}

#[tokio::test]
async fn rate_limits_record_the_fire_without_dispatching() {
    let (state, recorder) = app().await;
    let mut hook = input("Limited", TriggerSourceType::Webhook, json!({}));
    hook.rate_limit_seconds = Some(3600);
    let saved = create_trigger(&state, hook).await.unwrap();
    let (id, secret) = (saved.trigger.id, saved.secret.unwrap());

    state
        .triggers()
        .receive_webhook(&id, Some(&secret), json!({"n": 1}))
        .unwrap();
    wait_for_fires(&state, &id, 1).await;
    state
        .triggers()
        .receive_webhook(&id, Some(&secret), json!({"n": 2}))
        .unwrap();
    let list = wait_for_fires(&state, &id, 2).await;

    assert_eq!(list[0].status, TriggerFireStatus::Queued);
    assert_eq!(list[1].status, TriggerFireStatus::RateLimited);
    assert!(list[1].run_id.is_none());
    assert_eq!(count(&recorder, "trigger://rate_limited"), 1);
    // A manual test fire isn't rate limited
    let test = test_fire_trigger(&state, id, None).await.unwrap();
    assert_eq!(test.status, TriggerFireStatus::Queued);
    state.shutdown();
}

#[tokio::test]
async fn a_trigger_storm_disables_the_trigger() {
    let (state, recorder) = app().await;
    let saved = create_trigger(
        &state,
        input("Noisy", TriggerSourceType::Webhook, json!({})),
    )
    .await
    .unwrap();
    let (id, secret) = (saved.trigger.id, saved.secret.unwrap());
    for n in 0..6 {
        state
            .triggers()
            .receive_webhook(&id, Some(&secret), json!({ "n": n }))
            .unwrap();
    }
    let list = wait_for_fires(&state, &id, 6).await;
    assert!(list[..5]
        .iter()
        .all(|f| f.status == TriggerFireStatus::Queued));
    assert_eq!(list[5].status, TriggerFireStatus::Error);
    assert!(list[5].error.as_deref().unwrap().contains("disabled"));
    // Disabling happens in the background: allow for a loaded machine
    for _ in 0..500 {
        if !get_trigger(&state, id.clone()).await.unwrap().enabled {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(!get_trigger(&state, id.clone()).await.unwrap().enabled);
    assert!(count(&recorder, "trigger://error") >= 1);
    assert_eq!(
        state
            .triggers()
            .receive_webhook(&id, Some(&secret), json!({})),
        Err(ReceiveError::NotFound)
    );
    state.shutdown();
}

#[tokio::test]
async fn a_rate_limited_burst_is_one_run_not_a_storm() {
    let (state, _recorder) = app().await;
    let mut burst = input("Folder", TriggerSourceType::Webhook, json!({}));
    burst.rate_limit_seconds = Some(600);
    let saved = create_trigger(&state, burst).await.unwrap();
    let (id, secret) = (saved.trigger.id, saved.secret.unwrap());
    // Like a folder of files copied at once
    for n in 0..8 {
        state
            .triggers()
            .receive_webhook(&id, Some(&secret), json!({ "n": n }))
            .unwrap();
    }
    let list = wait_for_fires(&state, &id, 8).await;
    let queued = list
        .iter()
        .filter(|f| f.status == TriggerFireStatus::Queued)
        .count();
    assert_eq!(queued, 1);
    assert!(list.iter().all(|f| f.status != TriggerFireStatus::Error));
    assert!(get_trigger(&state, id).await.unwrap().enabled);
    state.shutdown();
}

#[tokio::test]
async fn device_events_fire_matching_triggers_even_while_the_agent_is_busy() {
    let (state, _recorder) = app().await;
    // The agent is mid-turn (suspended on an approval)
    let conversation = state
        .db()
        .call(|conn| db::ensure_agent_conversation(conn, "writer"))
        .await
        .unwrap();
    chat::submit(
        state.orchestrators(),
        "writer",
        conversation,
        "busy".into(),
        vec![],
        None,
        RunSource::user(),
    )
    .await
    .unwrap();

    let saved = create_trigger(
        &state,
        input(
            "Printer jam",
            TriggerSourceType::Device,
            json!({"deviceId": "printer-01", "events": ["paper_jam"]}),
        ),
    )
    .await
    .unwrap();
    let (id, secret) = (saved.trigger.id, saved.secret.unwrap());
    let engine = state.triggers();
    let event = |name: &str| DeviceEvent {
        event: name.into(),
        data: json!({"tray": 2}),
    };
    assert_eq!(
        engine.receive_device("printer-01", Some(&secret), event("ink_low")),
        Ok(0)
    );
    assert_eq!(
        engine.receive_device("printer-01", Some("nope"), event("paper_jam")),
        Err(ReceiveError::Unauthorized)
    );
    assert_eq!(
        engine.receive_device("scanner", Some(&secret), event("paper_jam")),
        Err(ReceiveError::NotFound)
    );
    assert_eq!(
        engine.receive_device("printer-01", Some(&secret), event("paper_jam")),
        Ok(1)
    );

    let fire = wait_for_fires(&state, &id, 1).await.remove(0);
    assert_eq!(fire.payload["event"], "paper_jam");
    assert_eq!(fire.payload["data"]["tray"], 2);
    let run = run(&state, fire.run_id.as_deref().unwrap()).await;
    assert_eq!(
        run.status,
        RunStatus::Queued,
        "waits behind the current turn"
    );
    assert_eq!(run.source_type, SourceType::Device);
    assert_eq!(run.priority, 50);
    state.shutdown();
}

#[tokio::test]
async fn event_triggers_chain_off_other_runs_but_not_their_own() {
    let (state, _recorder) = app().await;
    let mut retry = input(
        "Retry research",
        TriggerSourceType::Event,
        json!({"eventName": "agent.turn_done", "filter": {"agent_id": "writer", "status": "cancelled"}}),
    );
    retry.target_id = "fixer".into();
    retry.input_template = Some("Run {{payload.run_id}} ended {{payload.status}}".into());
    let id = create_trigger(&state, retry).await.unwrap().trigger.id;

    let conversation = state
        .db()
        .call(|conn| db::ensure_agent_conversation(conn, "writer"))
        .await
        .unwrap();
    let sent = chat::submit(
        state.orchestrators(),
        "writer",
        conversation,
        "research".into(),
        vec![],
        None,
        RunSource::user(),
    )
    .await
    .unwrap();
    chat::cancel_turn(&state, "writer".into(), sent.run_id.clone())
        .await
        .unwrap();

    let fire = wait_for_fires(&state, &id, 1).await.remove(0);
    assert_eq!(fire.status, TriggerFireStatus::Queued, "{:?}", fire.error);
    let chained = run(&state, fire.run_id.as_deref().unwrap()).await;
    assert_eq!(chained.agent_id, "fixer");
    assert_eq!(chained.source_type, SourceType::Event);
    assert_eq!(
        chained.input,
        format!("Run {} ended cancelled", sent.run_id)
    );

    // An event caused by this trigger's own run is skipped
    let own = test_fire_trigger(&state, id.clone(), Some(json!({"source_id": id})))
        .await
        .unwrap();
    assert_eq!(own.status, TriggerFireStatus::Skipped);
    assert!(own.run_id.is_none());
    state.shutdown();
}

#[tokio::test]
async fn rules_are_mirrored_to_yaml_and_loaded_from_the_repo() {
    let (state, recorder) = app().await;
    let dir = state.store().layout().triggers_dir.clone();
    let mut digest = input(
        "Daily Digest",
        TriggerSourceType::Cron,
        json!({"schedule": "0 9 * * 1-5"}),
    );
    digest.rate_limit_seconds = Some(3600);
    let saved = create_trigger(&state, digest.clone()).await.unwrap();
    assert_eq!(saved.trigger.id, "daily-digest");
    assert_eq!(saved.trigger.priority, 80);
    assert!(saved.secret.is_none());
    // A second rule with the same name gets its own id
    assert_eq!(
        create_trigger(&state, digest.clone())
            .await
            .unwrap()
            .trigger
            .id,
        "daily-digest-2"
    );
    assert!(count(&recorder, "trigger://changed") >= 2);

    let yaml = std::fs::read_to_string(dir.join("daily-digest.yaml")).unwrap();
    assert!(yaml.contains("schedule: 0 9 * * 1-5"), "{yaml}");

    // Updating keeps the id; switching to a webhook generates a secret
    let mut hook = digest.clone();
    hook.source_type = TriggerSourceType::Webhook;
    hook.source_config = json!({});
    let updated = update_trigger(&state, "daily-digest".into(), hook)
        .await
        .unwrap();
    assert_eq!(updated.trigger.id, "daily-digest");
    let secret = updated.secret.unwrap();
    let yaml = std::fs::read_to_string(dir.join("daily-digest.yaml")).unwrap();
    assert!(!yaml.contains(&secret), "secrets stay out of the repo");
    let rotated = rotate_trigger_secret(&state, "daily-digest".into())
        .await
        .unwrap();
    assert_ne!(rotated.secret.as_deref(), Some(secret.as_str()));

    delete_trigger(&state, "daily-digest-2".into())
        .await
        .unwrap();
    assert!(!dir.join("daily-digest-2.yaml").exists());
    assert!(delete_trigger(&state, "daily-digest-2".into())
        .await
        .is_err());

    // A file added to the repo (e.g. by a pull) wins on the next start
    std::fs::write(
        dir.join("from-repo.yaml"),
        "id: from-repo\nname: From repo\nsource:\n  type: event\n  config:\n    event_name: workflow.done\ntarget:\n  type: workflow\n  id: daily-report\n",
    )
    .unwrap();
    std::fs::remove_file(dir.join("daily-digest.yaml")).unwrap();
    state.shutdown();
    let restarted = TriggerEngine::new(
        state.db().clone(),
        state.bus().clone(),
        state.orchestrators().clone(),
        state.workflows().clone(),
        state.broadcast().clone(),
        tokio::runtime::Handle::current(),
        state.store().clone(),
        state.secrets().clone(),
    );
    Arc::new(restarted).start().await.unwrap();
    let ids: Vec<String> = list_triggers(&state)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.id)
        .collect();
    assert_eq!(ids, ["from-repo"]);
    let loaded = get_trigger(&state, "from-repo".into()).await.unwrap();
    assert_eq!(loaded.target_type, TriggerTargetType::Workflow);
    assert_eq!(loaded.priority, 60, "event triggers default to 60");
}

#[tokio::test]
async fn secrets_live_in_the_keyring_not_in_sqlite_or_files() {
    let (state, recorder) = app().await;
    let saved = create_trigger(&state, input("Hook", TriggerSourceType::Webhook, json!({})))
        .await
        .unwrap();
    let secret = saved.secret.unwrap();
    assert_eq!(
        state
            .secrets()
            .get("nandi.trigger.hook")
            .unwrap()
            .as_deref(),
        Some(secret.as_str())
    );
    let row: String = state
        .db()
        .call(|conn| {
            conn.query_row(
                "SELECT source_config FROM triggers WHERE id = 'hook'",
                [],
                |r| r.get(0),
            )
        })
        .await
        .unwrap();
    assert!(!row.contains(&secret), "{row}");
    let yaml = std::fs::read_to_string(state.store().layout().trigger_path("hook")).unwrap();
    assert!(!yaml.contains(&secret), "{yaml}");
    assert!(list_triggers(&state).await.unwrap()[0].has_secret);
    assert!(recorder.names().contains(&"repo://trigger.created"));

    // Requests still authenticate with it
    assert!(state
        .triggers()
        .receive_webhook("hook", Some(&secret), json!({}))
        .is_ok());

    delete_trigger(&state, "hook".into()).await.unwrap();
    assert_eq!(state.secrets().get("nandi.trigger.hook").unwrap(), None);
    assert!(recorder.names().contains(&"repo://trigger.deleted"));
    state.shutdown();
}

#[tokio::test]
async fn calendar_duties_are_tracked_and_handovers_skip_the_rate_limit() {
    use crate::services::domain::calendar::DutyId;
    use crate::services::relay::protocol::DutyState;

    let (state, _recorder) = app().await;
    let mut rule = input(
        "Support rota",
        TriggerSourceType::Calendar,
        json!({
            "times": ["09:00"], "people": ["alice", "bob"], "startDate": "2026-10-26",
            "backupAfterMinutes": 30
        }),
    );
    rule.rate_limit_seconds = Some(3600);
    let saved = create_trigger(&state, rule).await.unwrap();
    let id = saved.trigger.id.clone();
    assert!(state.triggers().is_watching(&id));

    // Long past and never fired: nobody ran it
    let duty = DutyId::parse(&format!("{id}/2026-10-27/09:00")).unwrap();
    assert_eq!(
        handover::duty_state(state.db(), &duty).await,
        DutyState::NotRun
    );

    let primary = json!({"duty_id": duty.to_string(), "role": "primary", "responsible": "alice", "backup": "bob"});
    let fire = state
        .triggers()
        .test_fire(&id, primary.clone())
        .await
        .unwrap();
    assert_eq!(fire.status, TriggerFireStatus::Queued);
    assert!(matches!(
        handover::duty_state(state.db(), &duty).await,
        DutyState::Running { .. } | DutyState::Completed | DutyState::Failed { .. }
    ));

    // A real (non-test) second fire inside the rate limit is held back,
    // but the backup taking over isn't
    let tx = state.triggers().tx.clone();
    for payload in [
        primary.clone(),
        calendar::handover_payload(&primary, "late"),
    ] {
        tx.send(TriggerEvent {
            trigger_id: id.clone(),
            source: TriggerSourceType::Calendar,
            payload,
            test: false,
        })
        .await
        .unwrap();
    }
    let list = wait_for_fires(&state, &id, 3).await;
    let status = |role: &str| {
        list.iter()
            .filter(|f| f.payload["role"] == role)
            .map(|f| f.status)
            .collect::<Vec<_>>()
    };
    assert!(status("primary").contains(&TriggerFireStatus::RateLimited));
    assert_eq!(status("backup"), [TriggerFireStatus::Queued]);
    state.shutdown();
}
