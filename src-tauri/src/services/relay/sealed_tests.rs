//! Sealing end to end, against a real relay: every frame between the
//! clients and the relay is recorded (a WebSocket tap in front of it), and
//! the content a task carries (its input, questions, answers, notes and
//! result) is never in one, nor in the relay's database. Reassignment
//! carries the requester's signature to the new provider, and a member with
//! no keys or a changed key is refused with a typed error.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;

use super::client::{ClientOptions, WsRelayClient};
use super::protocol::{PeerDelivery, PeerMessage, RelayMessage, TaskCompleted, TaskFailed};
use super::sealing::{PeerKeyBook, PEER_KEY_MISSING, SEAL_REFUSED};
use super::{RelayClient, RelaySealRefused};
use crate::db::Db;
use crate::services::a2a::{
    ClarificationRequest, ClarificationResponse, DelegationDirection, Question, Task, TaskResult,
    TaskStatus,
};
use crate::services::domain::agent_card::OwnerRef;
use crate::services::domain::capability::ServesCapability;
use crate::services::domain::AgentCard;
use crate::services::events::tests::{bus, RecordingSink};
use crate::services::network::peer_keys::{self, PEER_KEY_CHANGED};
use nandi_relay::{issue_instance_token, serve_with, RelayServerConfig};

const SECRET: &[u8] = b"sealed-secret";

/// In everything a task carries; must never be readable on the way
const MARKER: &str = "PLAINTEXT-MARKER-a91f04";

/// `file`'s bytes. On Windows the running relay's SQLite locks can refuse
/// a read for a moment (os error 33), so a locked read is tried again.
fn read_unlocked(file: &Path) -> Vec<u8> {
    for _ in 0..50 {
        match std::fs::read(file) {
            Err(e) if e.raw_os_error() == Some(33) => {
                std::thread::sleep(Duration::from_millis(20));
            }
            read => return read.unwrap(),
        }
    }
    std::fs::read(file).unwrap()
}

/// A WebSocket tap in front of a relay: it keeps every text frame either
/// way
struct Tap {
    addr: SocketAddr,
    frames: Arc<Mutex<Vec<String>>>,
}

impl Tap {
    async fn new(relay: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let frames: Arc<Mutex<Vec<String>>> = Arc::default();
        let kept = frames.clone();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(Self::pipe(stream, relay, kept.clone()));
            }
        });
        Self { addr, frames }
    }

    // tungstenite's handshake callback returns its own (large) error type
    #[allow(clippy::result_large_err)]
    async fn pipe(
        stream: tokio::net::TcpStream,
        relay: SocketAddr,
        frames: Arc<Mutex<Vec<String>>>,
    ) {
        let path: Arc<Mutex<String>> = Arc::default();
        let seen = path.clone();
        let Ok(client) =
            tokio_tungstenite::accept_hdr_async(stream, move |request: &Request, response| {
                *seen.lock().unwrap() = request.uri().to_string();
                Ok::<Response, _>(response)
            })
            .await
        else {
            return;
        };
        let target = format!("ws://{relay}{}", path.lock().unwrap());
        let Ok((upstream, _)) = tokio_tungstenite::connect_async(target).await else {
            return;
        };
        let (mut to_client, mut from_client) = client.split();
        let (mut to_relay, mut from_relay) = upstream.split();
        let (up_frames, down_frames) = (frames.clone(), frames);
        let up = async move {
            while let Some(Ok(message)) = from_client.next().await {
                if let Message::Text(text) = &message {
                    up_frames.lock().unwrap().push(text.to_string());
                }
                if to_relay.send(message).await.is_err() {
                    break;
                }
            }
        };
        let down = async move {
            while let Some(Ok(message)) = from_relay.next().await {
                if let Message::Text(text) = &message {
                    down_frames.lock().unwrap().push(text.to_string());
                }
                if to_client.send(message).await.is_err() {
                    break;
                }
            }
        };
        tokio::select! {
            _ = up => {}
            _ = down => {}
        }
    }

    fn frames(&self) -> Vec<String> {
        self.frames.lock().unwrap().clone()
    }
}

fn card(instance_id: &str, agent_id: &str, capability: &str) -> AgentCard {
    AgentCard {
        id: AgentCard::card_id(instance_id, agent_id),
        name: agent_id.to_string(),
        description: String::new(),
        instance_id: instance_id.to_string(),
        endpoint: String::new(),
        capabilities: vec![ServesCapability {
            name: capability.to_string(),
            description: String::new(),
            input_schema: None,
            output_schema: None,
        }],
        accepts_delegation: true,
        max_concurrent: 1,
        current_load: 0,
        owner: OwnerRef::default(),
        signature: None,
        issued_at: 0.0,
    }
}

/// One member's client, with what reached it
struct Member {
    client: Arc<WsRelayClient>,
    tasks: Arc<Mutex<Vec<Task>>>,
    peers: Arc<Mutex<Vec<PeerDelivery>>>,
    results: Arc<Mutex<Vec<TaskCompleted>>>,
    failures: Arc<Mutex<Vec<TaskFailed>>>,
    moves: Arc<Mutex<Vec<super::protocol::TaskEvent>>>,
    events: Arc<RecordingSink>,
}

impl Member {
    async fn join(
        tap: &Tap,
        instance_id: &str,
        cards: Vec<AgentCard>,
        options: ClientOptions,
    ) -> Self {
        let (bus, events) = bus();
        let client = Arc::new(WsRelayClient::with_options(Some(bus), instance_id, options));
        let token = issue_instance_token(SECRET, instance_id);
        client
            .connect(
                &format!("ws://{}/relay?token={token}", tap.addr),
                instance_id,
            )
            .await
            .unwrap();
        client.register(cards).await.unwrap();
        let member = Self {
            client,
            tasks: Arc::default(),
            peers: Arc::default(),
            results: Arc::default(),
            failures: Arc::default(),
            moves: Arc::default(),
            events,
        };
        let sink = member.tasks.clone();
        member
            .client
            .on_task_assigned(Box::new(move |assigned| {
                sink.lock().unwrap().push(assigned.task)
            }))
            .await;
        let sink = member.moves.clone();
        member
            .client
            .on_task_event(Box::new(move |event| sink.lock().unwrap().push(event)))
            .await;
        let sink = member.peers.clone();
        member
            .client
            .on_peer(Box::new(move |delivery| {
                sink.lock().unwrap().push(delivery)
            }))
            .await;
        let sink = member.results.clone();
        member
            .client
            .on_task_completed(Box::new(move |done| sink.lock().unwrap().push(done)))
            .await;
        let sink = member.failures.clone();
        member
            .client
            .on_task_failed(Box::new(move |failed| sink.lock().unwrap().push(failed)))
            .await;
        member
    }

    async fn task(&self) -> Task {
        wait(|| self.tasks.lock().unwrap().first().cloned()).await
    }

    async fn peer(&self, nth: usize) -> PeerMessage {
        wait(|| self.peers.lock().unwrap().get(nth).map(|d| d.body.clone())).await
    }

    fn events(&self) -> Vec<(&'static str, Value)> {
        self.events.0.lock().unwrap().clone()
    }

    fn refusals(&self) -> Vec<RelaySealRefused> {
        self.events()
            .into_iter()
            .filter(|(name, _)| *name == "relay://seal_refused")
            .map(|(_, payload)| serde_json::from_value(payload).unwrap())
            .collect()
    }
}

/// Polls `found` every 20ms for up to 5s
async fn wait<T>(mut found: impl FnMut() -> Option<T>) -> T {
    for _ in 0..250 {
        if let Some(value) = found() {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out");
}

fn request(id: &str, from: &str, target: &str) -> Task {
    Task {
        id: id.into(),
        requester: card(from, "asker", "ask"),
        target: target.into(),
        capability: "write".into(),
        input: json!({"brief": MARKER}),
        created_at: 1.0,
        deadline: None,
    }
}

fn result_of(task: &Task) -> TaskResult {
    TaskResult {
        task_id: task.id.clone(),
        provider_agent: task.target.clone(),
        output: json!({"text": format!("done: {MARKER}")}),
        status: TaskStatus::Completed,
        duration_ms: 3,
        tokens_in: 1,
        tokens_out: 2,
        error: None,
        direction: DelegationDirection::Inbound,
        capability: task.capability.clone(),
        requester_agent: task.requester.id.clone(),
        provider_owner: None,
        requester_owner: None,
        input: task.input.clone(),
        started_at: 1.0,
        ended_at: Some(4.0),
    }
}

/// Every file under `dir`
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(walk(&path));
        } else {
            files.push(path);
        }
    }
    files
}

fn company(data: &Path) -> RelayServerConfig {
    RelayServerConfig::new(SECRET.to_vec()).with_data_dir(data)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_its_questions_and_its_result_cross_the_relay_sealed() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve_with("127.0.0.1:0".parse().unwrap(), company(tmp.path()))
        .await
        .unwrap();
    let tap = Tap::new(relay).await;
    let bob = Member::join(
        &tap,
        "bob",
        vec![card("bob", "writer", "write")],
        ClientOptions::default(),
    )
    .await;
    let ada = Member::join(&tap, "ada", Vec::new(), ClientOptions::default()).await;

    // Ada's task reaches Bob opened
    let task = request("t1", "ada", "bob/writer");
    ada.client.delegate_task(&task, &[]).await.unwrap();
    // (the relay stamps when it routed it)
    let at_bob = bob.task().await;
    assert_eq!(
        Task {
            created_at: task.created_at,
            ..at_bob
        },
        task
    );

    // Bob acknowledges with a note and asks a question
    let ack = PeerMessage::AcknowledgeTask {
        task_id: "t1".into(),
        eta_seconds: Some(30),
        note: Some(format!("on it: {MARKER}")),
    };
    bob.client
        .send_peer("ada".into(), ack.clone())
        .await
        .unwrap();
    let question = PeerMessage::ClarificationRequest(ClarificationRequest {
        task_id: "t1".into(),
        round: 1,
        questions: vec![Question {
            id: "q1".into(),
            question: format!("Which {MARKER}?"),
            topic: "scope".into(),
            requires_human: true,
        }],
    });
    bob.client
        .send_peer("ada".into(), question.clone())
        .await
        .unwrap();
    assert_eq!(ada.peer(0).await, ack);
    assert_eq!(ada.peer(1).await, question);

    // Ada answers
    let answer = PeerMessage::ClarificationResponse(ClarificationResponse {
        task_id: "t1".into(),
        round: 1,
        answers: [("q1".to_string(), format!("the {MARKER}"))].into(),
    });
    ada.client
        .send_peer("bob".into(), answer.clone())
        .await
        .unwrap();
    assert_eq!(bob.peer(0).await, answer);

    // Bob's result comes back opened
    bob.client.send_result(result_of(&task)).await.unwrap();
    let done = wait(|| ada.results.lock().unwrap().first().cloned()).await;
    assert_eq!(
        done.result.output,
        json!({"text": format!("done: {MARKER}")})
    );
    assert_eq!(done.result.status, TaskStatus::Completed);
    assert!(ada.refusals().is_empty() && bob.refusals().is_empty());

    // Nothing of it crossed in the clear
    let frames = tap.frames();
    assert!(frames.len() > 10, "{} frames", frames.len());
    let sealed = frames
        .iter()
        .filter(|frame| frame.contains("\"ciphertext\""))
        .count();
    assert!(sealed >= 5, "{sealed} sealed frames");
    for frame in &frames {
        assert!(!frame.contains(MARKER), "a frame in the clear: {frame}");
    }
    for frame in frames {
        RelayMessage::from_json(&frame).expect("every frame is a relay message");
    }
    // Nor is it in the relay's database or WAL
    for file in walk(tmp.path()) {
        let bytes = read_unlocked(&file);
        assert!(
            !bytes.windows(MARKER.len()).any(|w| w == MARKER.as_bytes()),
            "{} holds the marker",
            file.display()
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reassigned_task_reaches_the_new_provider_signed_by_the_requester() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve_with("127.0.0.1:0".parse().unwrap(), company(tmp.path()))
        .await
        .unwrap();
    let tap = Tap::new(relay).await;
    let bob = Member::join(
        &tap,
        "bob",
        vec![card("bob", "writer", "write")],
        ClientOptions::default(),
    )
    .await;
    let cal = Member::join(
        &tap,
        "cal",
        vec![card("cal", "writer", "write")],
        ClientOptions::default(),
    )
    .await;
    let ada = Member::join(&tap, "ada", Vec::new(), ClientOptions::default()).await;

    let first = request("t1", "ada", "bob/writer");
    ada.client.delegate_task(&first, &[]).await.unwrap();
    bob.task().await;

    // Bob hands it to Cal: the relay moves it, the request resealed for Cal
    bob.client
        .reassign(
            "t1",
            "ada",
            "cal/writer",
            "Bob",
            Some(format!("swamped with {MARKER}")),
        )
        .await
        .unwrap();
    let at_cal = cal.task().await;
    assert_eq!(at_cal.id, "t1");
    assert_eq!(at_cal.input, first.input);
    let moved = wait(|| {
        ada.moves
            .lock()
            .unwrap()
            .iter()
            .find(|e| matches!(e, super::protocol::TaskEvent::Reassigned { .. }))
            .cloned()
    })
    .await;
    let super::protocol::TaskEvent::Reassigned {
        provider_instance,
        by,
        note,
        ..
    } = moved
    else {
        panic!("expected the move, got {moved:?}");
    };
    assert_eq!((provider_instance.as_str(), by.as_str()), ("cal", "Bob"));
    assert_eq!(note, Some(format!("swamped with {MARKER}")));
    // Bob can't speak for it any more; Cal answers Ada directly
    assert!(bob
        .client
        .reassign("t1", "ada", "bob/writer", "Bob", None)
        .await
        .is_err());
    cal.client.send_result(result_of(&at_cal)).await.unwrap();
    let done = wait(|| ada.results.lock().unwrap().first().cloned()).await;
    assert_eq!(done.task_id, "t1");
    assert!(cal.refusals().is_empty() && ada.refusals().is_empty());
    for frame in tap.frames() {
        assert!(!frame.contains(MARKER), "a frame in the clear: {frame}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_for_someone_offline_waits_at_the_relay_and_so_does_the_result() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve_with("127.0.0.1:0".parse().unwrap(), company(tmp.path()))
        .await
        .unwrap();
    let tap = Tap::new(relay).await;
    // Bob joins (his keys are pinned), then goes away
    let bob = Member::join(
        &tap,
        "bob",
        vec![card("bob", "writer", "write")],
        ClientOptions::default(),
    )
    .await;
    let keys = bob.client.clone();
    bob.client.connect("", "bob").await.unwrap();
    let ada = Member::join(&tap, "ada", Vec::new(), ClientOptions::default()).await;
    let task = request("t1", "ada", "bob/writer");
    assert!(ada.client.delegate_task(&task, &[]).await.unwrap().held);

    // Bob is back (the same device): the task was waiting for him
    let token = issue_instance_token(SECRET, "bob");
    keys.connect(&format!("ws://{}/relay?token={token}", tap.addr), "bob")
        .await
        .unwrap();
    keys.register(vec![card("bob", "writer", "write")])
        .await
        .unwrap();
    let at_bob = bob.task().await;
    assert_eq!(at_bob.input, task.input);
    // Ada goes away before the result; it waits for her
    ada.client.connect("", "ada").await.unwrap();
    keys.send_result(result_of(&at_bob)).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(ada.results.lock().unwrap().is_empty());
    let token = issue_instance_token(SECRET, "ada");
    ada.client
        .connect(&format!("ws://{}/relay?token={token}", tap.addr), "ada")
        .await
        .unwrap();
    ada.client.register(Vec::new()).await.unwrap();
    let done = wait(|| ada.results.lock().unwrap().first().cloned()).await;
    assert_eq!(done.task_id, "t1");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn content_for_a_member_without_keys_is_refused_before_it_leaves() {
    let relay = serve_with(
        "127.0.0.1:0".parse().unwrap(),
        RelayServerConfig::new(SECRET.to_vec()),
    )
    .await
    .unwrap();
    let tap = Tap::new(relay).await;
    let ada = Member::join(&tap, "ada", Vec::new(), ClientOptions::default()).await;
    let refused = ada
        .client
        .delegate_task(&request("t1", "ada", "nobody/writer"), &[])
        .await
        .unwrap_err();
    assert!(refused.starts_with(PEER_KEY_MISSING), "{refused}");
    // Nothing went out about it
    assert!(!tap
        .frames()
        .iter()
        .any(|frame| frame.contains("submit_task")));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_result_from_a_replaced_device_is_refused_and_its_key_flagged() {
    let tmp = tempfile::tempdir().unwrap();
    let relay = serve_with("127.0.0.1:0".parse().unwrap(), company(tmp.path()))
        .await
        .unwrap();
    let tap = Tap::new(relay).await;
    let db = Db::in_memory();
    let book: Arc<dyn PeerKeyBook> = Arc::new(peer_keys::PinnedKeys::new(db.clone()));
    let ada = Member::join(
        &tap,
        "ada",
        Vec::new(),
        ClientOptions {
            peer_keys: Some(book),
            ..ClientOptions::default()
        },
    )
    .await;
    let bob = Member::join(
        &tap,
        "bob",
        vec![card("bob", "writer", "write")],
        ClientOptions::default(),
    )
    .await;
    let task = request("t1", "ada", "bob/writer");
    ada.client.delegate_task(&task, &[]).await.unwrap();
    bob.task().await;
    assert!(peer_keys::pinned(&db, "bob").unwrap().is_some());

    // Bob's Nandi is reinstalled; the operator rekeys it
    bob.client.connect("", "bob").await.unwrap();
    let status = reqwest::Client::new()
        .post(format!("http://{relay}/relay/rekey"))
        .bearer_auth(std::str::from_utf8(SECRET).unwrap())
        .json(&json!({ "instanceId": "bob" }))
        .send()
        .await
        .unwrap()
        .status();
    assert!(status.is_success(), "{status}");
    let replaced = Member::join(
        &tap,
        "bob",
        vec![card("bob", "writer", "write")],
        ClientOptions::default(),
    )
    .await;

    // The new device answers: Ada can't verify it against the pinned key
    replaced.client.send_result(result_of(&task)).await.unwrap();
    let failed = wait(|| ada.failures.lock().unwrap().first().cloned()).await;
    assert_eq!(failed.task_id, "t1");
    assert!(failed.error.starts_with(SEAL_REFUSED), "{}", failed.error);
    assert!(ada.results.lock().unwrap().is_empty());
    let refusals = wait(|| Some(ada.refusals()).filter(|r| !r.is_empty())).await;
    assert_eq!(refusals[0].from_instance, "bob");
    // The change waits on Bob's profile, and sending to him is held
    wait(|| {
        peer_keys::status(&db, "bob")
            .unwrap()
            .filter(peer_keys::PeerKeyStatus::changed)
    })
    .await;
    let held = ada
        .client
        .send_peer(
            "bob".into(),
            PeerMessage::StatusQuery {
                task_id: "t1".into(),
            },
        )
        .await
        .unwrap_err();
    assert!(held.starts_with(PEER_KEY_CHANGED), "{held}");
    assert!(ada
        .events()
        .iter()
        .any(|(name, payload)| *name == "network://member_key_changed"
            && payload["instanceId"] == "bob"));
}
