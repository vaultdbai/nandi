//! Own devices on the real relay (`nandi_relay::testing`): this Nandi's
//! relay client, and a phone made the way the phone web app works (its own
//! keys, the claim endpoint, a device WebSocket and sealed envelopes).

use std::sync::Mutex;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use nandi_protocol::{DeviceClaimResponse, RelayMessage, PROTOCOL_VERSION};
use nandi_relay::RelayServerConfig;
use serde_json::json;
use tokio_tungstenite::tungstenite::Message;

use super::*;
use crate::model::PendingApproval;
use crate::services::a2a::relay_world::TestRelay;
use crate::services::relay::client::{ClientOptions, WsRelayClient};

const ME: &str = "ada-1";
const PIN: &str = "482916";

async fn wait_for<F, Fut>(what: &str, mut check: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..300 {
        if check().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("timed out waiting for {what}");
}

/// The Inbox, as a test sets it: its approvals, and the decisions made
#[derive(Default)]
struct FakeInbox {
    approvals: Mutex<Vec<PendingApproval>>,
    decided: Mutex<Vec<(String, ApprovalDecision, String)>>,
}

impl FakeInbox {
    fn waiting(&self, id: &str, tool: &str) {
        self.approvals.lock().unwrap().push(PendingApproval {
            id: id.into(),
            agent_id: "writer".into(),
            run_id: "r1".into(),
            conversation_id: "c1".into(),
            tool: tool.into(),
            args: json!({"path": "report.md"}),
            created_at: 1.0,
            expires_at: None,
        });
    }

    fn decided(&self) -> Vec<(String, ApprovalDecision, String)> {
        self.decided.lock().unwrap().clone()
    }
}

impl PhoneInbox for FakeInbox {
    fn approvals(
        &self,
    ) -> futures_util::future::BoxFuture<'_, Result<Vec<PendingApproval>, String>> {
        let approvals = self.approvals.lock().unwrap().clone();
        Box::pin(async move { Ok(approvals) })
    }

    fn todos(&self) -> futures_util::future::BoxFuture<'_, Result<Vec<PhoneTodo>, String>> {
        Box::pin(async {
            Ok(vec![PhoneTodo {
                id: "todo-1".into(),
                title: "Reply to the landlord".into(),
                kind: "do".into(),
                from: "Nandi".into(),
                due_at: None,
            }])
        })
    }

    fn decide(
        &self,
        id: String,
        decision: ApprovalDecision,
        device: String,
    ) -> futures_util::future::BoxFuture<'_, Result<(), String>> {
        self.approvals.lock().unwrap().retain(|a| a.id != id);
        self.decided.lock().unwrap().push((id, decision, device));
        Box::pin(async { Ok(()) })
    }
}

/// This Nandi on a test relay, with the own-devices service listening
struct Home {
    relay: TestRelay,
    _client: Arc<WsRelayClient>,
    devices: OwnDevices,
    keys: DeviceKeys,
    inbox: Arc<FakeInbox>,
}

impl Home {
    async fn on(config: RelayServerConfig) -> Self {
        let inbox = Arc::new(FakeInbox::default());
        Self::with(config, Db::in_memory(), inbox.clone(), inbox).await
    }

    /// On `db`, with `phone_inbox` as the Inbox the service decides in
    async fn with(
        config: RelayServerConfig,
        db: Db,
        phone_inbox: Arc<dyn PhoneInbox>,
        inbox: Arc<FakeInbox>,
    ) -> Self {
        let relay = TestRelay::start_with(config).await;
        let keys = DeviceKeys::generate().unwrap();
        let secret = keys.export_secret().to_string();
        let source: KeySource =
            Arc::new(move || DeviceKeys::import_secret(&secret).map_err(|e| e.to_string()));
        let options = ClientOptions {
            device_keys: Some(source.clone()),
            ..ClientOptions::default()
        };
        let client = relay.client_with(ME, options).await;
        client.register(Vec::new()).await.unwrap();
        let devices = OwnDevices::new(
            Some(client.clone() as Arc<dyn RelayClient>),
            db,
            ME.to_string(),
            source,
            None,
            phone_inbox,
        );
        let listener = devices.clone();
        let runtime = tokio::runtime::Handle::current();
        client
            .on_devices(Box::new(move |signal| {
                let devices = listener.clone();
                runtime.spawn(async move { devices.on_signal(signal).await });
            }))
            .await;
        Self {
            relay,
            _client: client,
            devices,
            keys,
            inbox,
        }
    }

    async fn start() -> Self {
        Self::on(RelayServerConfig::open()).await
    }

    async fn device(&self, device_id: &str) -> Option<DeviceView> {
        self.devices
            .overview()
            .await
            .unwrap()
            .devices
            .into_iter()
            .find(|d| d.device_id == device_id)
    }

    /// A phone that claimed a pairing code (not confirmed yet)
    async fn claimed_phone(&self) -> Phone {
        let pairing = self.devices.start_pairing("Ada's phone").await.unwrap();
        let phone = Phone::claim(&self.relay, &pairing.invite).await;
        let id = phone.device_id.clone();
        wait_for("the claim", || async {
            self.device(&id)
                .await
                .is_some_and(|d| d.status == DeviceStatus::Claimed)
        })
        .await;
        phone
    }

    /// A phone paired and confirmed with [`PIN`]
    async fn paired_phone(&self) -> Phone {
        let phone = self.claimed_phone().await;
        self.devices.confirm(&phone.device_id, PIN).await.unwrap();
        phone
    }
}

/// A phone, as the phone web app is one
struct Phone {
    keys: DeviceKeys,
    device_id: String,
    home: String,
    home_keys: PublicKeys,
    claim: DeviceClaimResponse,
    seq: u64,
}

impl Phone {
    async fn claim(relay: &TestRelay, invite: &PairingInvite) -> Self {
        let keys = DeviceKeys::generate().unwrap();
        let claim = nandi_relay::testing::claim_device(
            &relay.http_base(),
            &invite.code,
            keys.public_keys(),
        )
        .await
        .unwrap();
        assert_eq!(claim.device_id, invite.device_id);
        Self {
            keys,
            device_id: invite.device_id.clone(),
            home: invite.home_instance.clone(),
            home_keys: invite.home_keys.clone(),
            claim,
            seq: 0,
        }
    }

    /// `call` sealed for home, with the next `seq`
    fn seal(&mut self, call: serde_json::Value) -> Envelope {
        self.seq += 1;
        self.seal_at(self.seq, call)
    }

    fn seal_at(&self, seq: u64, call: serde_json::Value) -> Envelope {
        let sealed_for = SealedFor {
            task_id: self.device_id.clone(),
            seq,
            from: self.device_id.clone(),
            to: self.home.clone(),
            purpose: SealPurpose::OwnDevice,
            capability: String::new(),
        };
        let bytes = serde_json::to_vec(&call).unwrap();
        nandi_seal::seal(&self.keys, &self.home_keys, sealed_for, &bytes).unwrap()
    }

    /// Registers on the relay's WebSocket: its first answer
    async fn register(&self, relay: &TestRelay) -> (Socket, RelayMessage) {
        let (mut socket, _) = tokio_tungstenite::connect_async(relay.url()).await.unwrap();
        let register = RelayMessage::RegisterDevice {
            device_id: self.device_id.clone(),
            token: self.claim.token.clone(),
            protocol_version: PROTOCOL_VERSION,
        };
        socket
            .send(Message::text(serde_json::to_string(&register).unwrap()))
            .await
            .unwrap();
        let first = next_message(&mut socket).await;
        (socket, first)
    }

    /// Opens one of home's answers
    fn open(&self, envelope: &Envelope) -> Reply {
        let opened = nandi_seal::open(&self.keys, envelope, &self.home_keys).unwrap();
        assert_eq!(opened.sealed_for.purpose, SealPurpose::OwnDevice);
        serde_json::from_slice(&opened.content).unwrap()
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn next_message(socket: &mut Socket) -> RelayMessage {
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), socket.next())
            .await
            .expect("a frame in time")
            .unwrap()
            .unwrap();
        if let Message::Text(text) = frame {
            return RelayMessage::from_json(&text).unwrap();
        }
    }
}

fn answer_of(handled: Handled) -> Answer {
    match handled {
        Handled::Answered(reply) => reply.answer,
        Handled::Dropped(why) => panic!("dropped: {why}"),
    }
}

fn refusal_code(answer: &Answer) -> &str {
    match answer {
        Answer::Refused { code, .. } => code,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn pairing_shows_the_invite_then_the_phones_fingerprint_and_confirm_activates_it() {
    let home = Home::start().await;
    let pairing = home.devices.start_pairing("Ada's phone").await.unwrap();
    let invite = &pairing.invite;
    assert!(is_device_id(&invite.device_id));
    assert_eq!(invite.home_instance, ME);
    assert_eq!(&invite.home_keys, home.keys.public_keys());
    assert!(!invite.relay_url.contains("token"), "{}", invite.relay_url);
    let encoded = pairing.link.strip_prefix(PAIRING_LINK).unwrap();
    let decoded: PairingInvite =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).unwrap()).unwrap();
    assert_eq!(&decoded, invite);
    assert!(pairing.qr_svg.contains("<svg"), "{}", pairing.qr_svg);
    assert_eq!(
        home.device(&invite.device_id).await.unwrap().status,
        DeviceStatus::Pairing
    );

    let phone = Phone::claim(&home.relay, invite).await;
    let id = phone.device_id.clone();
    wait_for("the claim", || async {
        home.device(&id)
            .await
            .is_some_and(|d| d.status == DeviceStatus::Claimed)
    })
    .await;
    // The fingerprint the person compares with the phone's screen
    let claimed = home.device(&id).await.unwrap();
    assert_eq!(
        claimed.fingerprint,
        nandi_seal::fingerprint(phone.keys.public_keys()).ok()
    );
    // Not confirmed: the relay keeps the phone out
    let (_, first) = phone.register(&home.relay).await;
    assert!(
        matches!(&first, RelayMessage::Error { code, .. } if code == nandi_protocol::DEVICE_PENDING),
        "{first:?}"
    );

    // A weak PIN is refused; a good one confirms it
    assert!(home.devices.confirm(&id, "123456").await.is_err());
    let active = home.devices.confirm(&id, PIN).await.unwrap();
    assert_eq!(active.status, DeviceStatus::Active);
    assert!(active.paired_at.is_some());
    let (_, first) = phone.register(&home.relay).await;
    assert!(
        matches!(&first, RelayMessage::DeviceRegistered { home_instance, .. } if home_instance == ME),
        "{first:?}"
    );
    // Confirming twice does nothing
    assert!(home.devices.confirm(&id, PIN).await.is_err());
}

#[tokio::test]
async fn a_phone_reads_the_inbox_sealed_both_ways_through_the_relay() {
    let home = Home::start().await;
    home.inbox.waiting("a1", "write_file");
    home.inbox.waiting("a2", "send_email");
    let mut phone = home.paired_phone().await;
    let (mut socket, first) = phone.register(&home.relay).await;
    assert!(
        matches!(first, RelayMessage::DeviceRegistered { .. }),
        "{first:?}"
    );

    let envelope = phone.seal(json!({"id": "c1", "op": "inbox"}));
    socket
        .send(Message::text(
            serde_json::to_string(&RelayMessage::OwnEnvelope { envelope }).unwrap(),
        ))
        .await
        .unwrap();
    let reply = loop {
        if let RelayMessage::OwnDelivery { envelope, .. } = next_message(&mut socket).await {
            break phone.open(&envelope);
        }
    };
    assert_eq!(reply.re, "c1");
    let Answer::Inbox { approvals, todos } = reply.answer else {
        panic!("{:?}", reply.answer)
    };
    assert_eq!(approvals.len(), 2);
    assert_eq!(approvals[0].desktop_only, None);
    assert!(
        approvals[1].desktop_only.is_some(),
        "sending stays on the desktop"
    );
    assert_eq!(todos[0].title, "Reply to the landlord");
    // The relay saw it come and go, but nothing in it
    let seen = home.device(&phone.device_id).await.unwrap();
    assert!(seen.last_seen_at.is_some());
}

#[tokio::test]
async fn an_unconfirmed_or_unknown_device_is_never_answered() {
    let home = Home::start().await;
    let mut phone = home.claimed_phone().await;
    let envelope = phone.seal(json!({"id": "c1", "op": "inbox"}));
    assert_eq!(
        home.devices.on_envelope(envelope).await.unwrap(),
        Handled::Dropped("not confirmed on the desktop")
    );
    // One sealed by someone pretending to be a device we don't have
    let mut stranger = Phone {
        device_id: "device.ffffffffffffffffffffffffffffffff".into(),
        ..phone
    };
    let envelope = stranger.seal(json!({"id": "c2", "op": "inbox"}));
    assert!(matches!(
        home.devices.on_envelope(envelope).await.unwrap(),
        Handled::Dropped(_)
    ));
}

#[tokio::test]
async fn a_revoked_device_is_refused_at_once() {
    let home = Home::start().await;
    let mut phone = home.paired_phone().await;
    let envelope = phone.seal(json!({"id": "c1", "op": "inbox"}));
    assert!(matches!(
        home.devices.on_envelope(envelope).await.unwrap(),
        Handled::Answered(_)
    ));
    home.devices.revoke(&phone.device_id).await.unwrap();
    let envelope = phone.seal(json!({"id": "c2", "op": "inbox"}));
    assert_eq!(
        home.devices.on_envelope(envelope).await.unwrap(),
        Handled::Dropped("not one of our devices (or revoked)")
    );
    assert!(home.device(&phone.device_id).await.is_none());
    // The relay dropped it too: its token no longer registers
    let (_, first) = phone.register(&home.relay).await;
    assert!(matches!(first, RelayMessage::Error { .. }), "{first:?}");
}

#[tokio::test]
async fn a_replayed_or_forged_envelope_is_dropped() {
    let home = Home::start().await;
    let mut phone = home.paired_phone().await;
    let envelope = phone.seal(json!({"id": "c1", "op": "inbox"}));
    assert!(matches!(
        home.devices.on_envelope(envelope.clone()).await.unwrap(),
        Handled::Answered(_)
    ));
    assert_eq!(
        home.devices.on_envelope(envelope).await.unwrap(),
        Handled::Dropped("a replay")
    );
    // An older seq, freshly sealed, is a replay too
    let old = phone.seal_at(0, json!({"id": "c2", "op": "inbox"}));
    assert_eq!(
        home.devices.on_envelope(old).await.unwrap(),
        Handled::Dropped("a replay")
    );
    // Sealed by other keys than the ones confirmed
    let mut thief = Phone {
        keys: DeviceKeys::generate().unwrap(),
        ..phone
    };
    let forged = thief.seal(json!({"id": "c3", "op": "inbox"}));
    assert_eq!(
        home.devices.on_envelope(forged).await.unwrap(),
        Handled::Dropped("didn't open with the device's keys")
    );
}

#[tokio::test]
async fn every_approval_needs_the_pin_and_wrong_pins_lock_the_device() {
    let home = Home::start().await;
    home.inbox.waiting("a1", "write_file");
    let mut phone = home.paired_phone().await;
    let device_id = phone.device_id.clone();
    let mut approve =
        |pin: &str| phone.seal(json!({"id": "c", "op": "approve", "itemId": "a1", "pin": pin}));
    // No PIN: not a call a phone can make
    let without = approve("");
    let answer = answer_of(home.devices.on_envelope(without).await.unwrap());
    assert_eq!(refusal_code(&answer), codes::PIN_WRONG);

    for left in (1..pin::MAX_TRIES - 1).rev() {
        let answer = answer_of(home.devices.on_envelope(approve("000001")).await.unwrap());
        assert!(
            matches!(&answer, Answer::Refused { code, tries_left: Some(n), .. }
                if code == codes::PIN_WRONG && *n == left),
            "{answer:?}"
        );
    }
    let answer = answer_of(home.devices.on_envelope(approve("000001")).await.unwrap());
    assert_eq!(refusal_code(&answer), codes::DEVICE_LOCKED);
    // Locked: even the right PIN and the Inbox are refused
    let answer = answer_of(home.devices.on_envelope(approve(PIN)).await.unwrap());
    assert_eq!(refusal_code(&answer), codes::DEVICE_LOCKED);
    assert!(home.inbox.decided().is_empty());
    assert_eq!(
        home.device(&device_id).await.unwrap().status,
        DeviceStatus::Locked
    );

    // Only the desktop unlocks it
    home.devices.unlock(&device_id, None).await.unwrap();
    let answer = answer_of(home.devices.on_envelope(approve(PIN)).await.unwrap());
    assert_eq!(
        answer,
        Answer::Decided {
            item_id: "a1".into(),
            decision: "approved".into()
        }
    );
    // The record names the device
    let decided = home.inbox.decided();
    assert_eq!(decided.len(), 1);
    assert_eq!(decided[0].1, ApprovalDecision::Approved);
    assert_eq!(decided[0].2, format!("Ada's phone ({device_id})"));
}

#[tokio::test]
async fn risky_approvals_are_refused_from_a_device_but_may_be_declined() {
    let home = Home::start().await;
    home.inbox.waiting("a1", "send_email");
    home.inbox.waiting("a2", "connector_access");
    let mut phone = home.paired_phone().await;
    for id in ["a1", "a2"] {
        let call = json!({"id": "c", "op": "approve", "itemId": id, "pin": PIN});
        let answer = answer_of(home.devices.on_envelope(phone.seal(call)).await.unwrap());
        assert_eq!(refusal_code(&answer), codes::DESKTOP_ONLY, "{id}");
        assert!(
            matches!(&answer, Answer::Refused { message, .. } if message.contains("on your computer"))
        );
    }
    assert!(home.inbox.decided().is_empty());
    let decline = json!({"id": "c", "op": "decline", "itemId": "a1", "pin": PIN});
    let answer = answer_of(home.devices.on_envelope(phone.seal(decline)).await.unwrap());
    assert_eq!(
        answer,
        Answer::Decided {
            item_id: "a1".into(),
            decision: "declined".into()
        }
    );
    // Pairing, revoking or settings aren't calls a phone has
    let pair = json!({"id": "c", "op": "pair"});
    let answer = answer_of(home.devices.on_envelope(phone.seal(pair)).await.unwrap());
    assert_eq!(refusal_code(&answer), codes::UNKNOWN_COMMAND);
}

#[tokio::test]
async fn revoke_all_removes_every_device_here_and_at_the_relay() {
    let home = Home::start().await;
    let first = home.paired_phone().await;
    // Claimed but never confirmed here: it lapses at the relay
    let pairing = home.devices.start_pairing("Tablet").await.unwrap();
    assert_ne!(pairing.device_id, first.device_id);
    home.devices.revoke_all().await.unwrap();
    assert!(home.devices.overview().await.unwrap().devices.is_empty());
    let (_, answer) = first.register(&home.relay).await;
    assert!(matches!(answer, RelayMessage::Error { .. }), "{answer:?}");
}

#[tokio::test]
async fn without_a_relay_it_asks_to_connect_and_unlocking_still_works() {
    let keys = DeviceKeys::generate().unwrap();
    let secret = keys.export_secret().to_string();
    let devices = OwnDevices::new(
        Some(TestRelay::unconnected_client(ME, ClientOptions::default()) as Arc<dyn RelayClient>),
        Db::in_memory(),
        ME.into(),
        Arc::new(move || DeviceKeys::import_secret(&secret).map_err(|e| e.to_string())),
        None,
        Arc::new(FakeInbox::default()),
    );
    let overview = devices.overview().await.unwrap();
    assert!(!overview.connected && !overview.relay_needs_update);
    let error = devices.start_pairing("Phone").await.unwrap_err();
    assert!(RelayUnavailable::is(&error), "{error}");
    // Local: no such device, but no relay error either
    let error = devices.unlock("device.01", None).await.unwrap_err();
    assert!(!RelayUnavailable::is(&error), "{error}");
}

#[tokio::test]
async fn a_relay_one_version_back_says_it_needs_an_update_and_nothing_is_sent() {
    let before = nandi_protocol::MIN_SUPPORTED_PROTOCOL;
    if RelayFeature::OwnDevices.available_at(before) {
        return;
    }
    let home = Home::on(RelayServerConfig::open().with_protocol_version(before)).await;
    let overview = home.devices.overview().await.unwrap();
    assert!(overview.connected && overview.relay_needs_update);
    let error = home.devices.start_pairing("Phone").await.unwrap_err();
    assert!(RelayNeedsUpdate::is(&error), "{error}");
    assert_eq!(home.relay.frames_sent(ME, "device_request"), 0);
}

/// The whole path with the app's Inbox: a run's approval, approved from
/// the phone, is recorded as decided from that device
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_approval_from_the_phone_is_recorded_with_the_device() {
    let state = AppState::for_tests(Vec::new());
    crate::services::builtin_agents::seed(state.store().layout()).unwrap();
    state.store().load().unwrap();
    let script = json!({
        "agent": [
            { "text": "I'll note it.", "toolCalls": [{
                "tool": "write_file",
                "args": {"path": "notes/today.md", "content": "hi"},
                "result": "", "requiresApproval": true
            }] },
            { "text": "Done." }
        ],
        "studio": [],
        "tokenDelayMs": 0
    });
    state
        .db()
        .call(move |conn| {
            crate::db::set_setting(conn, crate::db::ENGINE_KEY, &json!("canned"))?;
            crate::db::set_setting(conn, crate::engine::CANNED_SCRIPT_KEY, &script)
        })
        .await
        .unwrap();
    let agent = crate::services::builtin_agents::DEFAULT_AGENT;
    crate::services::chat::send_message(&state, agent.into(), "Note it".into(), vec![], None)
        .await
        .unwrap();
    let pending = || async {
        crate::services::chat::list_pending_approvals(&state)
            .await
            .unwrap()
    };
    wait_for("the approval", || async { !pending().await.is_empty() }).await;
    let approval = pending().await.remove(0);

    let home = Home::with(
        RelayServerConfig::open(),
        state.db().clone(),
        Arc::new(AppInbox::new(state.clone())),
        Arc::new(FakeInbox::default()),
    )
    .await;
    let mut phone = home.paired_phone().await;

    let call = json!({"id": "c1", "op": "approve", "itemId": approval.id, "pin": PIN});
    let answer = answer_of(home.devices.on_envelope(phone.seal(call)).await.unwrap());
    assert!(matches!(answer, Answer::Decided { .. }), "{answer:?}");
    let month = chrono::Utc::now().format("%Y-%m").to_string();
    let record = || {
        state
            .store()
            .repos()
            .decisions
            .read(&month)
            .unwrap()
            .unwrap_or_default()
    };
    wait_for("the decision record", || async {
        record().contains("**Device:**")
    })
    .await;
    assert!(
        record().contains(&format!("- **Device:** Ada's phone ({})", phone.device_id)),
        "{}",
        record()
    );
    state.shutdown();
}
