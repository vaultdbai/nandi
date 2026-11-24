//! The person's own devices (protocol 8): a phone paired with this Nandi
//! through the relay, which may see the Inbox and approve or decline from
//! anywhere. Settings › Your devices. See `docs/developer/own-devices.md`
//! for the contract the phone web app follows.
//!
//! - **Pairing** happens only here. [`OwnDevices::start_pairing`] asks the
//!   relay for a device and a single-use code (`DeviceOp::Pair`) and shows
//!   a QR code of the [`PairingInvite`]. The phone claims the code with
//!   its own keys; the relay tells us (`DeviceClaimed`), and the screen
//!   shows the phone's key fingerprint. Once the person sees it matches
//!   the phone's screen and picks a PIN, [`OwnDevices::confirm`] sends
//!   `DeviceOp::Confirm` with those keys and keeps the PIN as a slow hash
//!   ([`pin`]).
//! - **Envelopes** from a device are opened here with the device's
//!   confirmed keys ([`OwnDevices::on_envelope`]): one from an unknown,
//!   unconfirmed or revoked device, sealed for someone else or not signed
//!   by the device, or with a `seq` at or below the last one taken (a
//!   replay), is dropped unanswered. The content is a small command set
//!   ([`wire`]): the Inbox's approvals and to-dos, approve, decline.
//! - **Every approve and decline carries the PIN**, checked here. Wrong
//!   PINs count; [`pin::MAX_TRIES`] in a row lock the device until the
//!   desktop unlocks it. The riskiest approvals are refused from a device
//!   ([`desktop_only`]): "approve this on your computer". The decisions
//!   record names the device (`inbox::audit::on_device`).
//! - **Revoking** removes the device here first (its envelopes are
//!   dropped at once), then at the relay. A device the relay has that this
//!   Nandi doesn't know is revoked there when the list is read.
//!
//! Without a relay the calls answer `RelayUnavailable` (unlocking is
//! local and still works); a relay one version older answers
//! `RelayNeedsUpdate` and nothing is sent ([`RelayFeature::OwnDevices`]).

pub mod desktop_only;
pub mod inbox;
pub mod pin;
pub mod store;
pub mod wire;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use nandi_protocol::{
    is_device_id, DeviceOp, DeviceOutcome, DeviceState, Envelope, OwnDevice, PairingInvite,
    PublicKeys, SealPurpose, SealedFor, DEVICE_NOT_FOUND,
};
use nandi_seal::DeviceKeys;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::db::{now_ms, Db};
use crate::model::ApprovalDecision;
use crate::services::events::{self, EventBus, NandiEvent};
use crate::services::relay::client_api::DeviceSignal;
use crate::services::relay::device_keys::DeviceKeyring;
use crate::services::relay::features::RelayNeedsUpdate;
use crate::services::relay::{RelayClient, RelayFeature, RelayUnavailable};
use crate::services::AppState;

use inbox::{AppInbox, PhoneInbox};
use store::{DeviceRow, RowState};
use wire::{codes, Answer, Command, PhoneApproval, PhoneTodo, Read, Reply};

/// Where a phone opens its pairing: the invite rides in the fragment, so
/// it never reaches the site's server
pub const PAIRING_LINK: &str = "https://vaultdb.ai/pair#";

/// The longest device name, in characters
const MAX_NAME_CHARS: usize = 40;

/// `devices://changed`: a device was claimed, confirmed, locked, unlocked
/// or revoked
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct OwnDevicesChanged {
    pub device_id: Option<String>,
}

impl NandiEvent for OwnDevicesChanged {
    const NAME: &'static str = "devices://changed";
}

/// Where a device stands, as Settings › Your devices shows it
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum DeviceStatus {
    /// The QR code is shown; no phone claimed it yet
    Pairing,
    /// A phone claimed it: compare its fingerprint, then confirm
    Claimed,
    Active,
    /// Too many wrong PINs: unlock it here
    Locked,
}

/// One device in Settings › Your devices
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DeviceView {
    pub device_id: String,
    pub name: String,
    pub status: DeviceStatus,
    /// The phone's key fingerprint (`3F2A 91C0 7B44 E1D9`), once claimed
    pub fingerprint: Option<String>,
    pub paired_at: Option<f64>,
    /// When this Nandi or the relay last heard from it
    pub last_seen_at: Option<f64>,
    pub online: bool,
    pub failed_pins: u32,
}

/// Settings › Your devices
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DevicesOverview {
    /// `false`: no relay (the screen asks to connect)
    pub connected: bool,
    /// The relay is too old for devices (the screen says it needs an update)
    pub relay_needs_update: bool,
    pub devices: Vec<DeviceView>,
}

/// A pairing just opened: shown once
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PairingView {
    pub device_id: String,
    pub name: String,
    pub invite: PairingInvite,
    /// What the QR code holds: [`PAIRING_LINK`] and the invite
    pub link: String,
    /// The QR code, as an SVG document
    pub qr_svg: String,
    pub expires_at: f64,
}

/// What became of one envelope from a device
#[derive(Debug, Clone, PartialEq)]
pub enum Handled {
    Answered(Reply),
    /// Not answered, and why
    Dropped(&'static str),
}

/// This instance's keys
pub type KeySource = Arc<dyn Fn() -> Result<DeviceKeys, String> + Send + Sync>;

/// Envelopes are handled one at a time, so `seq` and the PIN count can't race
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The next `seq` of an envelope we send: increasing, from the clock in
/// microseconds, so it keeps rising across restarts
fn next_seq() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = u64::try_from(chrono::Utc::now().timestamp_micros()).unwrap_or_default();
    let step = |last: u64| last.saturating_add(1).max(now);
    let previous = LAST
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |last| {
            Some(step(last))
        })
        .unwrap_or_default();
    step(previous)
}

/// How the decisions record names a device
fn label(row: &DeviceRow) -> String {
    format!("{} ({})", row.name, row.device_id)
}

fn fingerprint(keys: &PublicKeys) -> Option<String> {
    nandi_seal::fingerprint(keys).ok()
}

/// The person's devices, over one relay connection and `state.db`
#[derive(Clone)]
pub struct OwnDevices {
    relay: Option<Arc<dyn RelayClient>>,
    db: Db,
    me: String,
    keys: KeySource,
    bus: Option<EventBus>,
    inbox: Arc<dyn PhoneInbox>,
}

impl OwnDevices {
    pub fn new(
        relay: Option<Arc<dyn RelayClient>>,
        db: Db,
        me: String,
        keys: KeySource,
        bus: Option<EventBus>,
        inbox: Arc<dyn PhoneInbox>,
    ) -> Self {
        Self {
            relay,
            db,
            me,
            keys,
            bus,
            inbox,
        }
    }

    pub fn of(state: &AppState) -> Result<Self, String> {
        let keyring = Arc::new(DeviceKeyring::new(state.secrets().clone()));
        Ok(Self::new(
            state.relay_client().cloned(),
            state.db().clone(),
            crate::services::network::local_instance_id(state.db())?,
            Arc::new(move || keyring.device_keys()),
            Some(state.bus().clone()),
            Arc::new(AppInbox::new(state.clone())),
        ))
    }

    fn emit(&self, device_id: Option<&str>) {
        if let Some(bus) = &self.bus {
            events::emit(
                bus,
                &OwnDevicesChanged {
                    device_id: device_id.map(str::to_string),
                },
            );
        }
    }

    /// The relay, connected and new enough for devices
    fn relay(&self) -> Result<&Arc<dyn RelayClient>, String> {
        let relay = self
            .relay
            .as_ref()
            .filter(|relay| relay.status().connected)
            .ok_or_else(|| String::from(RelayUnavailable))?;
        if relay
            .status()
            .missing_features
            .contains(&RelayFeature::OwnDevices)
        {
            return Err(RelayNeedsUpdate.into());
        }
        Ok(relay)
    }

    async fn call(&self, op: DeviceOp) -> Result<DeviceOutcome, String> {
        self.relay()?.device_request(op).await
    }

    async fn rows(&self) -> Result<Vec<DeviceRow>, String> {
        self.db.call(|conn| store::list(conn)).await
    }

    async fn row(&self, device_id: &str) -> Result<Option<DeviceRow>, String> {
        let id = device_id.to_string();
        self.db.call(move |conn| store::get(conn, &id)).await
    }

    // -----------------------------------------------------------------
    // The desktop
    // -----------------------------------------------------------------

    /// Settings › Your devices. Lapsed codes and claims go; a device the
    /// relay has and this Nandi doesn't confirm is revoked there; one the
    /// relay no longer has goes here.
    pub async fn overview(&self) -> Result<DevicesOverview, String> {
        let relay = match self.relay() {
            Ok(relay) => relay,
            Err(e) => {
                return Ok(DevicesOverview {
                    connected: !RelayUnavailable::is(&e),
                    relay_needs_update: RelayNeedsUpdate::is(&e),
                    devices: Vec::new(),
                })
            }
        };
        let now = now_ms();
        self.db
            .call(move |conn| store::remove_lapsed(conn, now))
            .await?;
        let remote = match relay.device_request(DeviceOp::List).await? {
            DeviceOutcome::Devices { devices } => devices,
            DeviceOutcome::Refused { message, .. } => return Err(message),
            other => {
                return Err(format!(
                    "The relay answered something unexpected: {other:?}"
                ))
            }
        };
        let mut rows = self.rows().await?;
        self.reconcile(&mut rows, &remote).await;
        let devices = rows
            .iter()
            .map(|row| view(row, remote.iter().find(|d| d.device_id == row.device_id)))
            .collect();
        Ok(DevicesOverview {
            connected: true,
            relay_needs_update: false,
            devices,
        })
    }

    /// Brings our rows and the relay's list together (see [`Self::overview`])
    async fn reconcile(&self, rows: &mut Vec<DeviceRow>, remote: &[OwnDevice]) {
        for device in remote {
            let ours = rows.iter().find(|row| row.device_id == device.device_id);
            match (ours, device.state) {
                // A claim whose push we missed
                (Some(row), DeviceState::Claimed) if row.state != RowState::Active => {
                    if let Some(keys) = &device.public_keys {
                        let (id, keys) = (device.device_id.clone(), keys.clone());
                        let expires = device.expires_at.map(|at| at as i64);
                        let _ = self
                            .db
                            .call(move |conn| store::claimed(conn, &id, &keys, expires))
                            .await;
                    }
                }
                (Some(_), _) => {}
                // Active at the relay, unknown or unconfirmed here: we can't
                // check its PIN, so it goes
                (None, DeviceState::Active) => {
                    log::info!("Own devices: revoking {} (not ours)", device.device_id);
                    let _ = self
                        .call(DeviceOp::Revoke {
                            device_id: device.device_id.clone(),
                        })
                        .await;
                }
                (None, _) => {}
            }
        }
        for row in rows.iter() {
            let at_relay = remote.iter().any(|d| d.device_id == row.device_id);
            if !at_relay && row.state != RowState::Pairing {
                let id = row.device_id.clone();
                let _ = self.db.call(move |conn| store::remove(conn, &id)).await;
            }
        }
        if let Ok(fresh) = self.rows().await {
            *rows = fresh;
        }
    }

    /// Opens a pairing for a device the person calls `name`: the QR code
    /// to show, valid for a few minutes and only once
    pub async fn start_pairing(&self, name: &str) -> Result<PairingView, String> {
        let name = name.trim();
        let name = if name.is_empty() { "Phone" } else { name };
        if name.chars().count() > MAX_NAME_CHARS {
            return Err(format!(
                "A device's name has at most {MAX_NAME_CHARS} characters"
            ));
        }
        let relay = self.relay()?.clone();
        let home_keys = (self.keys)()?.public_keys().clone();
        let relay_url = relay
            .status()
            .without_token()
            .relay_url
            .ok_or_else(|| String::from(RelayUnavailable))?;
        let (device_id, code, expires_at) = match self.call(DeviceOp::Pair).await? {
            DeviceOutcome::Pairing {
                device_id,
                code,
                expires_at,
            } => (device_id, code, expires_at),
            DeviceOutcome::Refused { code, message } => return Err(refusal(&code, &message)),
            other => {
                return Err(format!(
                    "The relay answered something unexpected: {other:?}"
                ))
            }
        };
        let (id, row_name) = (device_id.clone(), name.to_string());
        let now = now_ms();
        self.db
            .call(move |conn| store::open_pairing(conn, &id, &row_name, now, expires_at as i64))
            .await?;
        let invite = PairingInvite {
            relay_url,
            home_instance: self.me.clone(),
            home_keys,
            device_id: device_id.clone(),
            code,
            expires_at,
        };
        let json = serde_json::to_vec(&invite).map_err(|e| e.to_string())?;
        let link = format!("{PAIRING_LINK}{}", URL_SAFE_NO_PAD.encode(json));
        let qr_svg = qr_svg(&link)?;
        self.emit(Some(&device_id));
        Ok(PairingView {
            device_id,
            name: name.to_string(),
            invite,
            link,
            qr_svg,
            expires_at,
        })
    }

    /// The relay says a phone claimed one of our codes
    pub async fn on_claimed(&self, device: OwnDevice) {
        let Some(keys) = device.public_keys.clone() else {
            return;
        };
        let id = device.device_id.clone();
        let expires = device.expires_at.map(|at| at as i64);
        match self
            .db
            .call(move |conn| store::claimed(conn, &id, &keys, expires))
            .await
        {
            Ok(true) => self.emit(Some(&device.device_id)),
            Ok(false) => log::info!("Own devices: a claim for {} isn't ours", device.device_id),
            Err(e) => log::warn!("Own devices: {e}"),
        }
    }

    /// The person saw the fingerprints match and chose `pin`: the device
    /// may now reach this Nandi
    pub async fn confirm(&self, device_id: &str, pin: &str) -> Result<DeviceView, String> {
        if let Some(problem) = pin::problem(pin) {
            return Err(problem);
        }
        let row = self
            .row(device_id)
            .await?
            .filter(|row| row.state == RowState::Claimed)
            .ok_or("No phone waits to be confirmed for this pairing. Start again")?;
        let keys = row.keys.clone().ok_or("This phone hasn't sent its keys")?;
        let pin = pin.to_string();
        let hash = tokio::task::spawn_blocking(move || pin::hash(&pin))
            .await
            .map_err(|e| e.to_string())??;
        match self
            .call(DeviceOp::Confirm {
                device_id: device_id.to_string(),
                public_keys: keys,
            })
            .await?
        {
            DeviceOutcome::Done => {}
            DeviceOutcome::Refused { code, message } => return Err(refusal(&code, &message)),
            other => {
                return Err(format!(
                    "The relay answered something unexpected: {other:?}"
                ))
            }
        }
        let id = device_id.to_string();
        let now = now_ms();
        self.db
            .call(move |conn| store::activate(conn, &id, &hash, now))
            .await?;
        self.emit(Some(device_id));
        let row = self.row(device_id).await?.ok_or("The device went")?;
        Ok(view(&row, None))
    }

    /// Removes one device: here at once, then at the relay
    pub async fn revoke(&self, device_id: &str) -> Result<(), String> {
        let id = device_id.to_string();
        self.db.call(move |conn| store::remove(conn, &id)).await?;
        self.emit(Some(device_id));
        match self
            .call(DeviceOp::Revoke {
                device_id: device_id.to_string(),
            })
            .await?
        {
            DeviceOutcome::Refused { code, .. } if code == DEVICE_NOT_FOUND => Ok(()),
            DeviceOutcome::Refused { code, message } => Err(refusal(&code, &message)),
            _ => Ok(()),
        }
    }

    /// Removes every device: here at once, then at the relay
    pub async fn revoke_all(&self) -> Result<(), String> {
        self.db.call(|conn| store::remove_all(conn)).await?;
        self.emit(None);
        match self.call(DeviceOp::RevokeAll).await? {
            DeviceOutcome::Refused { code, message } => Err(refusal(&code, &message)),
            _ => Ok(()),
        }
    }

    /// Unlocks a device the wrong PINs locked, with a new PIN if one was
    /// chosen. Local: works without the relay.
    pub async fn unlock(&self, device_id: &str, new_pin: Option<String>) -> Result<(), String> {
        let hash = match new_pin.filter(|p| !p.is_empty()) {
            Some(pin) => {
                if let Some(problem) = pin::problem(&pin) {
                    return Err(problem);
                }
                Some(
                    tokio::task::spawn_blocking(move || pin::hash(&pin))
                        .await
                        .map_err(|e| e.to_string())??,
                )
            }
            None => None,
        };
        let id = device_id.to_string();
        let done = self
            .db
            .call(move |conn| store::unlock(conn, &id, hash.as_deref()))
            .await?;
        if !done {
            return Err("No such paired device".to_string());
        }
        self.emit(Some(device_id));
        Ok(())
    }

    // -----------------------------------------------------------------
    // A device's envelopes
    // -----------------------------------------------------------------

    /// Opens, checks and answers one envelope the relay forwarded
    pub async fn on_envelope(&self, envelope: Envelope) -> Result<Handled, String> {
        let _one = ONE_AT_A_TIME.lock().await;
        let sealed = envelope.sealed_for.clone();
        let device_id = sealed.from.clone();
        if sealed.purpose != SealPurpose::OwnDevice
            || sealed.to != self.me
            || sealed.task_id != device_id
            || !sealed.capability.is_empty()
            || !is_device_id(&device_id)
        {
            return Ok(Handled::Dropped("not an own-device envelope for us"));
        }
        let Some(row) = self.row(&device_id).await? else {
            return Ok(Handled::Dropped("not one of our devices (or revoked)"));
        };
        if row.state != RowState::Active {
            return Ok(Handled::Dropped("not confirmed on the desktop"));
        }
        let Some(keys) = row.keys.clone() else {
            return Ok(Handled::Dropped("no keys"));
        };
        let me = (self.keys)()?;
        let Ok(opened) = nandi_seal::open(&me, &envelope, &keys) else {
            return Ok(Handled::Dropped("didn't open with the device's keys"));
        };
        let (id, seq, now) = (device_id.clone(), sealed.seq, now_ms());
        if !self
            .db
            .call(move |conn| store::take_seq(conn, &id, seq, now))
            .await?
        {
            return Ok(Handled::Dropped("a replay"));
        }
        let (re, answer) = match wire::read(&opened.content) {
            Read::Unreadable => return Ok(Handled::Dropped("unreadable")),
            Read::Unknown { id } => (
                id,
                Answer::refused(
                    codes::UNKNOWN_COMMAND,
                    "A phone can see the Inbox and approve or decline. Do the rest on your computer.",
                ),
            ),
            Read::Call(call) => (call.id, self.answer(&row, call.command).await),
        };
        let reply = Reply { re, answer };
        self.send(&me, &row, &keys, &reply).await;
        Ok(Handled::Answered(reply))
    }

    async fn send(&self, me: &DeviceKeys, row: &DeviceRow, keys: &PublicKeys, reply: &Reply) {
        let Ok(bytes) = serde_json::to_vec(reply) else {
            return;
        };
        let sealed_for = SealedFor {
            task_id: row.device_id.clone(),
            seq: next_seq(),
            from: self.me.clone(),
            to: row.device_id.clone(),
            purpose: SealPurpose::OwnDevice,
            capability: String::new(),
        };
        let envelope = match nandi_seal::seal(me, keys, sealed_for, &bytes) {
            Ok(envelope) => envelope,
            Err(e) => return log::warn!("Own devices: couldn't seal the answer: {e}"),
        };
        if let Some(relay) = &self.relay {
            if let Err(e) = relay.send_own_envelope(envelope).await {
                log::info!("Own devices: the answer didn't go: {e}");
            }
        }
    }

    async fn answer(&self, row: &DeviceRow, command: Command) -> Answer {
        if row.is_locked() {
            return locked();
        }
        match command {
            Command::Inbox => self.inbox_answer().await,
            Command::Approve { item_id, pin } => {
                self.decide(row, item_id, &pin, ApprovalDecision::Approved)
                    .await
            }
            Command::Decline { item_id, pin } => {
                self.decide(row, item_id, &pin, ApprovalDecision::Rejected)
                    .await
            }
        }
    }

    async fn inbox_answer(&self) -> Answer {
        let approvals = match self.inbox.approvals().await {
            Ok(approvals) => approvals,
            Err(e) => return Answer::refused(codes::FAILED, e),
        };
        let todos = self.inbox.todos().await.unwrap_or_default();
        Answer::Inbox {
            approvals: approvals
                .into_iter()
                .take(wire::MAX_ITEMS)
                .map(|a| PhoneApproval {
                    desktop_only: desktop_only::reason(&a.tool, &a.args).map(str::to_string),
                    title: wire::short(&a.tool, wire::MAX_TITLE_CHARS),
                    detail: wire::short(&a.args.to_string(), wire::MAX_DETAIL_CHARS),
                    id: a.id,
                    agent: a.agent_id,
                    created_at: a.created_at,
                    expires_at: a.expires_at,
                })
                .collect(),
            todos: todos
                .into_iter()
                .take(wire::MAX_ITEMS)
                .collect::<Vec<PhoneTodo>>(),
        }
    }

    /// The PIN first: nothing is decided, or told about the item, before it
    async fn decide(
        &self,
        row: &DeviceRow,
        item_id: String,
        pin: &str,
        decision: ApprovalDecision,
    ) -> Answer {
        match self.check_pin(row, pin).await {
            Ok(None) => {}
            Ok(Some(refused)) => return refused,
            Err(e) => return Answer::refused(codes::FAILED, e),
        }
        let approvals = match self.inbox.approvals().await {
            Ok(approvals) => approvals,
            Err(e) => return Answer::refused(codes::FAILED, e),
        };
        let Some(approval) = approvals.into_iter().find(|a| a.id == item_id) else {
            return Answer::refused(codes::ITEM_NOT_FOUND, "Nothing waits for that any more");
        };
        if decision == ApprovalDecision::Approved {
            if let Some(why) = desktop_only::reason(&approval.tool, &approval.args) {
                return Answer::refused(
                    codes::DESKTOP_ONLY,
                    format!("Approve this on your computer. {why}."),
                );
            }
        }
        match self
            .inbox
            .decide(item_id.clone(), decision, label(row))
            .await
        {
            Ok(()) => Answer::Decided {
                item_id,
                decision: match decision {
                    ApprovalDecision::Approved => "approved",
                    ApprovalDecision::Rejected => "declined",
                }
                .to_string(),
            },
            Err(e) => Answer::refused(codes::FAILED, e),
        }
    }

    /// `None` when `pin` is the device's; else the refusal to answer
    async fn check_pin(&self, row: &DeviceRow, pin: &str) -> Result<Option<Answer>, String> {
        let stored = row.pin_hash.clone().unwrap_or_default();
        let typed = pin.to_string();
        let right = tokio::task::spawn_blocking(move || pin::matches(&typed, &stored))
            .await
            .map_err(|e| e.to_string())?;
        let id = row.device_id.clone();
        if right {
            self.db
                .call(move |conn| store::right_pin(conn, &id))
                .await?;
            return Ok(None);
        }
        let now = now_ms();
        let failed = self
            .db
            .call(move |conn| store::wrong_pin(conn, &id, pin::MAX_TRIES, now))
            .await?;
        self.emit(Some(&row.device_id));
        if failed >= pin::MAX_TRIES {
            return Ok(Some(locked()));
        }
        Ok(Some(Answer::Refused {
            code: codes::PIN_WRONG.to_string(),
            message: "That PIN isn't right".to_string(),
            tries_left: Some(pin::MAX_TRIES - failed),
        }))
    }

    /// Hands one relay signal to its handler
    pub async fn on_signal(&self, signal: DeviceSignal) {
        match signal {
            DeviceSignal::Claimed(device) => self.on_claimed(device).await,
            DeviceSignal::Delivered { envelope, .. } => match self.on_envelope(envelope).await {
                Ok(Handled::Dropped(why)) => log::info!("Own devices: dropped an envelope: {why}"),
                Ok(Handled::Answered(_)) => {}
                Err(e) => log::warn!("Own devices: {e}"),
            },
        }
    }
}

fn locked() -> Answer {
    Answer::refused(
        codes::DEVICE_LOCKED,
        "Too many wrong PINs. Unlock this phone on your computer: Settings › Your devices.",
    )
}

fn view(row: &DeviceRow, remote: Option<&OwnDevice>) -> DeviceView {
    let status = match row.state {
        RowState::Pairing => DeviceStatus::Pairing,
        RowState::Claimed => DeviceStatus::Claimed,
        RowState::Active if row.is_locked() => DeviceStatus::Locked,
        RowState::Active => DeviceStatus::Active,
    };
    let seen_here = row.last_seen_at.map(|at| at as f64);
    let seen_there = remote.and_then(|d| d.last_seen_at);
    DeviceView {
        device_id: row.device_id.clone(),
        name: row.name.clone(),
        status,
        fingerprint: row.keys.as_ref().and_then(fingerprint),
        paired_at: row.paired_at.map(|at| at as f64),
        last_seen_at: match (seen_here, seen_there) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        },
        online: remote.is_some_and(|d| d.online),
        failed_pins: row.failed_pins,
    }
}

/// A relay refusal as the screen says it
fn refusal(code: &str, message: &str) -> String {
    match code {
        nandi_protocol::DEVICE_LIMIT => {
            format!("You have as many devices as this relay allows. Revoke one first ({message})")
        }
        nandi_protocol::DEVICE_KEYS_MISMATCH => {
            "The phone's keys changed since it claimed the code. Start the pairing again"
                .to_string()
        }
        _ if message.trim().is_empty() => format!("The relay refused it ({code})"),
        _ => message.to_string(),
    }
}

/// `text` as a QR code, an SVG document
fn qr_svg(text: &str) -> Result<String, String> {
    let code = qrcode::QrCode::new(text.as_bytes())
        .map_err(|e| format!("Couldn't make the QR code: {e}"))?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(240, 240)
        .dark_color(qrcode::render::svg::Color("#000000"))
        .light_color(qrcode::render::svg::Color("#ffffff"))
        .build())
}

/// Hands the relay's own-device news to the service
pub fn watch(state: &AppState, client: &Arc<dyn RelayClient>) {
    let task_state = state.clone();
    let runtime = state.runtime().clone();
    let handler = Box::new(move |signal: DeviceSignal| {
        let state = task_state.clone();
        runtime.spawn(async move {
            match OwnDevices::of(&state) {
                Ok(devices) => devices.on_signal(signal).await,
                Err(e) => log::warn!("Own devices: {e}"),
            }
        });
    });
    let client = client.clone();
    state
        .runtime()
        .spawn(async move { client.on_devices(handler).await });
}
