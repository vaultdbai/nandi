use std::collections::HashMap;

use nandi_protocol::{SealedTask, TaskStatus};
use serde_json::json;

use super::*;
use crate::services::a2a::Question;
use crate::services::domain::AgentCard;
use crate::services::network::peer_keys::PEER_KEY_CHANGED;

fn user_card(instance: &str) -> AgentCard {
    AgentCard {
        id: AgentCard::card_id(instance, "_user"),
        name: "Nandi user".into(),
        description: String::new(),
        instance_id: instance.into(),
        endpoint: String::new(),
        capabilities: Vec::new(),
        accepts_delegation: false,
        max_concurrent: 0,
        current_load: 0,
        owner: Default::default(),
        signature: None,
        issued_at: 1.0,
    }
}

fn sealer(me: &str) -> Sealer {
    Sealer::new(me, DeviceKeys::generate().unwrap())
}

fn task(id: &str, target: &str) -> Task {
    Task {
        id: id.into(),
        requester: user_card("ada"),
        target: target.into(),
        capability: "write".into(),
        input: json!({"brief": "the secret plan"}),
        created_at: 1.0,
        deadline: Some(9.0),
    }
}

fn sealed_task(ada: &Sealer, task: &Task, to: (&str, &Sealer)) -> SealedTask {
    SealedTask {
        id: task.id.clone(),
        requester: task.requester.clone(),
        target: task.target.clone(),
        capability: task.capability.clone(),
        created_at: task.created_at,
        deadline: task.deadline,
        envelope: ada.seal_task(task, (to.0, to.1.public_keys())).unwrap(),
    }
}

fn opened<T>(opening: Opening<T>) -> T {
    match opening {
        Opening::Opened(value) => value,
        Opening::Duplicate => panic!("dropped as a duplicate"),
    }
}

fn result_of(task: &Task) -> TaskResult {
    TaskResult {
        task_id: task.id.clone(),
        provider_agent: task.target.clone(),
        output: json!({"text": "the answer"}),
        status: TaskStatus::Completed,
        duration_ms: 5,
        tokens_in: 1,
        tokens_out: 2,
        error: Some("a warning".into()),
        direction: DelegationDirection::Inbound,
        capability: task.capability.clone(),
        requester_agent: task.requester.id.clone(),
        provider_owner: None,
        requester_owner: None,
        input: task.input.clone(),
        started_at: 1.0,
        ended_at: Some(6.0),
    }
}

#[test]
fn a_task_forgotten_after_opening_opens_again() {
    let (ada, bob) = (sealer("ada"), sealer("bob"));
    let task = task("t1", "bob/writer");
    let sealed = sealed_task(&ada, &task, ("bob", &bob));
    opened(bob.open_task(sealed.clone(), ada.public_keys()).unwrap());
    // Not kept (its row wasn't stored): the relay's next delivery opens
    bob.forget(&sealed.envelope);
    assert_eq!(
        opened(bob.open_task(sealed.clone(), ada.public_keys()).unwrap()),
        task
    );
    assert_eq!(
        bob.open_task(sealed, ada.public_keys()).unwrap(),
        Opening::Duplicate
    );
}

#[test]
fn a_task_and_its_result_open_only_for_their_recipient() {
    let (ada, bob, eve) = (sealer("ada"), sealer("bob"), sealer("eve"));
    let task = task("t1", "bob/writer");
    let sealed = sealed_task(&ada, &task, ("bob", &bob));
    assert!(!serde_json::to_string(&sealed)
        .unwrap()
        .contains("secret plan"));
    assert_eq!(
        opened(bob.open_task(sealed.clone(), ada.public_keys()).unwrap()),
        task
    );
    // Eve can't open it, and Bob drops it the second time
    assert!(eve
        .open_task(sealed.clone(), ada.public_keys())
        .unwrap_err()
        .starts_with(SEAL_REFUSED));
    assert_eq!(
        bob.open_task(sealed, ada.public_keys()).unwrap(),
        Opening::Duplicate
    );

    let report = bob
        .seal_result(&result_of(&task), ("ada", ada.public_keys()))
        .unwrap();
    assert!(!serde_json::to_string(&report)
        .unwrap()
        .contains("the answer"));
    let back = opened(ada.open_result(report, ("bob", bob.public_keys())).unwrap());
    assert_eq!(back.output, json!({"text": "the answer"}));
    assert_eq!(back.error.as_deref(), Some("a warning"));
    assert_eq!(back.status, TaskStatus::Completed);
    assert_eq!(back.input, serde_json::Value::Null);
}

#[test]
fn a_forged_author_or_a_moved_envelope_is_refused() {
    let (ada, bob, mallory) = (sealer("ada"), sealer("bob"), sealer("mallory"));
    let task = task("t1", "bob/writer");
    // Mallory seals a task claiming Ada's card: Bob checks Ada's key
    let forged = SealedTask {
        envelope: Sealer::new("ada", DeviceKeys::generate().unwrap())
            .seal_task(&task, ("bob", bob.public_keys()))
            .unwrap(),
        ..sealed_task(&mallory, &task, ("bob", &bob))
    };
    let refused = bob.open_task(forged, ada.public_keys()).unwrap_err();
    assert!(refused.starts_with(SEAL_REFUSED), "{refused}");

    // An envelope moved to another task, or another capability
    let mut moved = sealed_task(&ada, &task, ("bob", &bob));
    moved.id = "t2".into();
    assert!(bob
        .open_task(moved, ada.public_keys())
        .unwrap_err()
        .contains("another task"));
    let mut other = sealed_task(&ada, &task, ("bob", &bob));
    other.capability = "delete".into();
    assert!(bob
        .open_task(other, ada.public_keys())
        .unwrap_err()
        .contains("another capability"));
    // A note passed off as a result
    let note = bob
        .seal_peer(
            "ada",
            PeerMessage::AcknowledgeTask {
                task_id: "t1".into(),
                eta_seconds: None,
                note: Some("hi".into()),
            },
            Some(ada.public_keys()),
        )
        .unwrap();
    let wire::PeerMessage::AcknowledgeTask {
        envelope: Some(envelope),
        ..
    } = note
    else {
        panic!("expected a sealed note");
    };
    let mut report = bob
        .seal_result(&result_of(&task), ("ada", ada.public_keys()))
        .unwrap();
    report.envelope = envelope;
    assert!(ada
        .open_result(report, ("bob", bob.public_keys()))
        .unwrap_err()
        .contains("holds a note"));
}

#[test]
fn every_peer_message_round_trips_with_its_content_sealed() {
    let (ada, bob) = (sealer("ada"), sealer("bob"));
    let bodies = vec![
        PeerMessage::AcknowledgeTask {
            task_id: "t1".into(),
            eta_seconds: Some(9),
            note: Some("on it, the secret plan".into()),
        },
        PeerMessage::RejectTask {
            task_id: "t1".into(),
            reason: "the secret plan is too big".into(),
            reason_kind: Some(wire::ReasonKind::NotAccepting),
        },
        PeerMessage::ProgressUpdate {
            task_id: "t1".into(),
            percent: Some(40),
            note: Some("the secret plan, halfway".into()),
            reason_kind: None,
            queue_position: Some(2),
        },
        PeerMessage::StatusQuery {
            task_id: "t1".into(),
        },
        PeerMessage::StatusResponse {
            task_id: "t1".into(),
            state: wire::TaskState::Completed,
            eta_seconds: None,
            elapsed_seconds: 4,
        },
        PeerMessage::CancelTask {
            task_id: "t1".into(),
            reason: Some("the secret plan changed".into()),
        },
        PeerMessage::TaskDelivered {
            task_id: "t1".into(),
        },
        PeerMessage::ClarificationRequest(ClarificationRequest {
            task_id: "t1".into(),
            round: 1,
            questions: vec![Question {
                id: "q1".into(),
                question: "Which secret plan?".into(),
                topic: "scope".into(),
                requires_human: true,
            }],
        }),
        PeerMessage::ClarificationResponse(ClarificationResponse {
            task_id: "t1".into(),
            round: 1,
            answers: HashMap::from([("q1".to_string(), "the secret plan".to_string())]),
        }),
        PeerMessage::ClarificationTimeout {
            task_id: "t1".into(),
        },
    ];
    for body in bodies {
        let wire = bob
            .seal_peer("ada", body.clone(), Some(ada.public_keys()))
            .unwrap();
        let text = serde_json::to_string(&wire).unwrap();
        assert!(!text.contains("secret plan"), "{text}");
        assert_eq!(
            Sealer::has_content(&body),
            !wire.envelopes().is_empty(),
            "{body:?}"
        );
        let back = opened(
            ada.open_peer("bob", wire.clone(), Some(bob.public_keys()))
                .unwrap(),
        );
        assert_eq!(back, body);
        // A second delivery is dropped; one without content opens again
        let again = ada
            .open_peer("bob", wire.clone(), Some(bob.public_keys()))
            .unwrap();
        if wire.envelopes().is_empty() {
            assert_eq!(again, Opening::Opened(body));
        } else {
            assert_eq!(again, Opening::Duplicate);
        }
    }
}

#[test]
fn content_needs_the_targets_keys_and_a_sender_s_own_envelope() {
    let (ada, bob, eve) = (sealer("ada"), sealer("bob"), sealer("eve"));
    let ask = || {
        PeerMessage::ClarificationRequest(ClarificationRequest {
            task_id: "t1".into(),
            round: 1,
            questions: Vec::new(),
        })
    };
    assert!(bob
        .seal_peer("ada", ask(), None)
        .unwrap_err()
        .starts_with(PEER_KEY_MISSING));
    // Without content nothing is sealed, keys or not
    bob.seal_peer(
        "ada",
        PeerMessage::StatusQuery {
            task_id: "t1".into(),
        },
        None,
    )
    .unwrap();
    // Bob's envelope, relayed as if Eve sent it
    let wire = bob
        .seal_peer("ada", ask(), Some(ada.public_keys()))
        .unwrap();
    let refused = ada
        .open_peer("eve", wire, Some(eve.public_keys()))
        .unwrap_err();
    assert!(refused.contains("sealed by someone else"), "{refused}");
}

#[test]
fn content_over_the_limit_is_refused_before_sealing() {
    let (ada, bob) = (sealer("ada"), sealer("bob"));
    let mut big = task("t1", "bob/writer");
    big.input = json!("x".repeat(MAX_ENVELOPE_BYTES));
    let refused = ada.seal_task(&big, ("bob", bob.public_keys())).unwrap_err();
    assert!(refused.starts_with(ENVELOPE_TOO_LARGE), "{refused}");
    // Just under it seals
    let mut fits = task("t1", "bob/writer");
    fits.input = json!("x".repeat(MAX_ENVELOPE_BYTES / 4 * 3 - 200));
    assert!(ada
        .seal_task(&fits, ("bob", bob.public_keys()))
        .unwrap()
        .is_within_limit());
}

#[test]
fn a_reassigned_request_keeps_the_requesters_signature() {
    let (ada, bob, cal) = (sealer("ada"), sealer("bob"), sealer("cal"));
    let first = task("t1", "bob/writer");
    opened(
        bob.open_task(sealed_task(&ada, &first, ("bob", &bob)), ada.public_keys())
            .unwrap(),
    );
    assert!(bob.holds_request("t1"));

    // Bob reseals it for Cal; the relay delivers it under the same task
    let request = bob
        .reseal_request("t1", ("cal", cal.public_keys()))
        .unwrap()
        .expect("Bob holds the request");
    assert_eq!(request.sealed_for.task_id, "t1");
    assert_eq!(request.sealed_for.to, "cal");
    assert_eq!(request.sealed_for.from, "bob");
    let moved = SealedTask {
        target: "cal/writer".into(),
        envelope: request,
        ..sealed_task(&ada, &first, ("cal", &cal))
    };
    // Only Ada's signature opens it, and only under its own task
    let fresh = sealer("ada");
    assert!(cal
        .open_task(moved.clone(), fresh.public_keys())
        .unwrap_err()
        .contains("isn't signed by its author"));
    assert!(cal
        .open_task(
            SealedTask {
                id: "t9".into(),
                ..moved.clone()
            },
            ada.public_keys(),
        )
        .unwrap_err()
        .contains("another task"));
    let at_cal = opened(cal.open_task(moved, ada.public_keys()).unwrap());
    assert_eq!(at_cal.id, "t1");
    assert_eq!(at_cal.input, first.input);
    assert!(cal.holds_request("t1"));

    // Without the opened request (a restart), no reseal: Ada seals again
    let restarted = sealer("bob");
    assert!(restarted
        .reseal_request("t1", ("cal", cal.public_keys()))
        .unwrap()
        .is_none());
}

#[test]
fn a_duty_error_is_a_note_bound_to_the_duty() {
    let (pia, ben) = (sealer("pia"), sealer("ben"));
    let duty = "standup/2026-11-04/09:00";
    let envelope = pia
        .note(
            duty,
            ("ben", ben.public_keys()),
            Some("the secret plan crashed".into()),
        )
        .unwrap()
        .unwrap();
    assert!(!serde_json::to_string(&envelope).unwrap().contains("secret"));
    let text = opened(
        ben.open_note(Some(&envelope), duty, ("pia", pia.public_keys()))
            .unwrap(),
    );
    assert_eq!(text.as_deref(), Some("the secret plan crashed"));
    assert!(ben
        .open_note(
            Some(&envelope),
            "other/2026-11-04/09:00",
            ("pia", pia.public_keys())
        )
        .is_err());
}

#[test]
fn the_memory_book_pins_once_and_refuses_a_change() {
    let book = MemoryKeyBook::default();
    let (first, second) = (sealer("a"), sealer("b"));
    assert_eq!(book.pinned("bob").unwrap(), None);
    book.learn("bob", first.public_keys()).unwrap();
    assert_eq!(
        book.pinned("bob").unwrap().as_ref(),
        Some(first.public_keys())
    );
    let changed = book.learn("bob", second.public_keys()).unwrap_err();
    assert!(changed.starts_with(PEER_KEY_CHANGED), "{changed}");
}

#[test]
fn sequence_numbers_increase_and_differ_per_envelope() {
    let (ada, bob) = (sealer("ada"), sealer("bob"));
    let a = ada
        .seal_task(&task("t1", "bob/w"), ("bob", bob.public_keys()))
        .unwrap();
    let b = ada
        .seal_task(&task("t1", "bob/w"), ("bob", bob.public_keys()))
        .unwrap();
    assert!(b.sealed_for.seq > a.sealed_for.seq);
    // A later Sealer (a restart) starts past them
    std::thread::sleep(std::time::Duration::from_millis(2));
    let later = sealer("ada");
    let c = later
        .seal_task(&task("t1", "bob/w"), ("bob", bob.public_keys()))
        .unwrap();
    assert!(c.sealed_for.seq > b.sealed_for.seq);
}
