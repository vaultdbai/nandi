//! The trial's modes and what the gateway last said (no network here; the
//! gateway client has its own tests against a fake gateway)

use super::*;
use serde_json::Map;

/// A port that was free a moment ago: nothing answers there
fn unreachable_url() -> String {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    format!("http://127.0.0.1:{port}")
}

fn trial() -> Trial {
    Trial::new(unreachable_url(), "test-instance".into())
}

fn api(code: &str, retry_after: Option<u64>) -> GatewayError {
    GatewayError::Api(Box::new(gateway::ApiFailure {
        status: 400,
        code: code.into(),
        message: String::new(),
        retry_after,
        fields: Map::new(),
    }))
}

#[test]
fn without_a_trial_nothing_is_offered_or_written() {
    let secrets = Secrets::in_memory();
    assert!(!available_with(None, &secrets));
    assert!(!chosen_with(None, &secrets));
    assert_eq!(why_unavailable_with(None, &secrets), None);
    assert!(set_enabled_with(None, &secrets, true).is_err());
    // Tests run with no trial unless they set one
    assert!(!available(&secrets));
    assert_eq!(status(&secrets, false), None);
    own_model_set_up(&secrets);
    own_model_found_with(false, &secrets);
    assert_eq!(secrets.get(MODE_ENTRY).unwrap(), None);
}

#[test]
fn an_own_model_turns_the_trial_off_until_the_user_turns_it_on() {
    let secrets = Secrets::in_memory();
    let trial = trial();
    assert_eq!(mode(&secrets), TrialMode::Auto);
    assert!(available_with(Some(&trial), &secrets));
    assert!(!chosen_with(Some(&trial), &secrets));

    // A key or sign-in found: off for good, even once it's gone
    own_model_found_with(true, &secrets);
    assert_eq!(mode(&secrets), TrialMode::Off);
    assert!(!available_with(Some(&trial), &secrets));

    // The user turns it back on; finding their model again keeps it on
    set_enabled_with(Some(&trial), &secrets, true).unwrap();
    assert!(chosen_with(Some(&trial), &secrets));
    own_model_found_with(true, &secrets);
    assert_eq!(mode(&secrets), TrialMode::On);

    // Saving another key turns it off again
    own_model_set_up_with(true, &secrets);
    assert_eq!(mode(&secrets), TrialMode::Off);
    assert!(!chosen_with(Some(&trial), &secrets));

    // And the user can turn it off themselves
    set_enabled_with(Some(&trial), &secrets, true).unwrap();
    set_enabled_with(Some(&trial), &secrets, false).unwrap();
    assert_eq!(mode(&secrets), TrialMode::Off);
}

#[test]
fn an_unreadable_mode_counts_as_off() {
    let secrets = Secrets::in_memory();
    secrets.make_unreadable(MODE_ENTRY);
    assert_eq!(mode(&secrets), TrialMode::Off);
    assert!(!available_with(Some(&trial()), &secrets));
}

#[test]
fn used_up_on_the_gateway_stops_the_trial_and_it_cannot_be_turned_on() {
    let secrets = Secrets::in_memory();
    let trial = trial();
    trial.note::<()>(&Err(api("trial_used_up", None)));
    assert!(!available_with(Some(&trial), &secrets));
    assert_eq!(
        why_unavailable_with(Some(&trial), &secrets),
        Some(TRIAL_USED_UP)
    );
    assert!(set_enabled_with(Some(&trial), &secrets, true)
        .unwrap_err()
        .contains("used up"));

    // The gateway's count is the authority: tokens left again, usable again
    trial.note_account(&TrialAccount {
        account: Some("google:1".into()),
        email: Some("ada@example.com".into()),
        used: 10,
        limit: 100_000,
        paused: false,
    });
    assert!(available_with(Some(&trial), &secrets));
}

#[test]
fn paused_and_unreachable_are_not_offered_for_a_while() {
    let secrets = Secrets::in_memory();
    let trial = trial();
    trial.note::<()>(&Err(api("trial_paused", Some(3_600))));
    assert!(!available_with(Some(&trial), &secrets));
    assert_eq!(
        why_unavailable_with(Some(&trial), &secrets),
        Some(TRIAL_PAUSED)
    );
    // A pause that's over is offered again
    trial.health().paused_until = Some(Instant::now() - Duration::from_secs(1));
    assert!(available_with(Some(&trial), &secrets));

    trial.note::<()>(&Err(GatewayError::Unreachable("refused".into())));
    assert_eq!(
        why_unavailable_with(Some(&trial), &secrets),
        Some(TRIAL_OFFLINE)
    );
    // An answer clears it
    trial.note(&Ok(()));
    assert!(available_with(Some(&trial), &secrets));

    // Other refusals don't take the trial away
    trial.note::<()>(&Err(api("rate_limited", Some(30))));
    trial.note::<()>(&Err(api("trial_not_enough", None)));
    assert!(available_with(Some(&trial), &secrets));
}

#[test]
fn a_paused_answer_from_the_status_route_is_rechecked_later() {
    let secrets = Secrets::in_memory();
    let trial = trial();
    trial.note_account(&TrialAccount {
        account: None,
        email: None,
        used: 0,
        limit: 100_000,
        paused: true,
    });
    assert!(!available_with(Some(&trial), &secrets));
    // A turned-off trial isn't explained: the user's choice is the reason
    set_mode(&secrets, TrialMode::Off).unwrap();
    assert_eq!(why_unavailable_with(Some(&trial), &secrets), None);
}

#[test]
fn turning_the_trial_off_gives_back_the_old_default() {
    let secrets = Secrets::in_memory();
    remember_default(&secrets, "claude:claude-sonnet-5");
    assert_eq!(
        take_remembered_default(&secrets).as_deref(),
        Some("claude:claude-sonnet-5")
    );
    // Only once, and the trial's own model is never remembered
    assert_eq!(take_remembered_default(&secrets), None);
    remember_default(&secrets, TRIAL_MODEL);
    assert_eq!(take_remembered_default(&secrets), None);
}

#[tokio::test]
async fn a_test_trial_is_used_by_this_thread_only() {
    let secrets = Secrets::in_memory();
    use_test_trial(Some(Arc::new(trial())));
    assert!(available(&secrets));
    let status = status(&secrets, true).unwrap();
    assert!(status.enabled && status.active && !status.used_up);
    // Nothing listens there: the call fails clearly and the trial rests
    let error = chat(
        &secrets,
        &reqwest::Client::new(),
        &LlmRequest {
            model: TRIAL_MODEL.into(),
            system: None,
            messages: vec![crate::llm::Message::User {
                content: "hi".into(),
            }],
            tools: vec![],
            max_tokens: None,
        },
        64,
        None,
    )
    .await
    .unwrap_err();
    assert!(error.contains("Can't reach the free trial"), "{error}");
    assert!(!available(&secrets));
    assert!(status_unreachable(&secrets));
    use_test_trial(None);
    assert!(!available(&secrets));
}

fn status_unreachable(secrets: &Secrets) -> bool {
    status(secrets, false).is_some_and(|s| s.unreachable)
}
