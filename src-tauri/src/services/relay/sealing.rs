//! Sealing what this Nandi sends to another person, and opening what it
//! gets: the only place content meets the wire. The relay client
//! (`client.rs`) calls it for every task, result and peer message; the rest
//! of the app sees the opened forms (`opened`).
//!
//! | Out | Sealed for | Signed by |
//! | --- | --- | --- |
//! | A task's input | The provider's pinned keys | Us (the requester) |
//! | A result's output and error | The requester's pinned keys | Us (the provider) |
//! | Notes, reasons, questions, answers, duty errors | The other side's pinned keys | Us |
//! | A reassigned task's request | The new provider's pinned keys | The requester, kept by [`nandi_seal::reseal`] |
//!
//! Opening checks, in order, and refuses with [`SEAL_REFUSED`] when one
//! fails: the envelope is within [`MAX_ENVELOPE_BYTES`], it's for us, for
//! this task and purpose (and a request or result for its capability),
//! from the instance that sent it, not one we already opened (a
//! duplicate `(task, from, seq)` is dropped, [`Opening::Duplicate`]), and
//! the seal opens with the author's pinned signing key. The caller finds
//! the keys: pinned in `network::peer_keys`, else asked of the relay and
//! pinned on first sight; a member with no keys anywhere is
//! [`PEER_KEY_MISSING`], one whose key changed `PEER_KEY_CHANGED`.
//!
//! Local, in-process delegation (our own agents) never comes here.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use nandi_protocol::{
    self as wire, AnswerContent, ClarificationContent, ClarificationRequest, ClarificationResponse,
    DelegationDirection, Envelope, NoteContent, PublicKeys, RequestContent, ResultContent,
    ResultReport, SealedContent, SealedFor, SealedTask, Task, TaskResult, LIFECYCLE_CAPABILITY,
    MAX_ENVELOPE_BYTES,
};
use nandi_seal::{DeviceKeys, Opened};

use super::opened::PeerMessage;

/// The stable code a refusal to send to a member with no keys starts with
pub const PEER_KEY_MISSING: &str = "peer_key_missing";

/// The stable code an envelope that didn't open (or didn't belong where
/// it came) is refused with
pub const SEAL_REFUSED: &str = "seal_refused";

/// The stable code content too big to seal is refused with (files go as
/// links)
pub const ENVELOPE_TOO_LARGE: &str = wire::ENVELOPE_TOO_LARGE;

/// `(task, from, seq)` remembered to drop duplicates
const SEEN_KEPT: usize = 8192;

/// Opened requests kept to reseal on a reassignment
const REQUESTS_KEPT: usize = 1024;

/// Bytes the seal adds around the content: the author's key ID (16), the
/// signature (64) and the AEAD tag (16)
const SEAL_OVERHEAD: usize = 16 + 64 + 16;

/// The encoded `enc`: 32 bytes as unpadded base64url
const ENC_CHARS: usize = 43;

/// Other instances' pinned keys (`network::peer_keys` over the database)
pub trait PeerKeyBook: Send + Sync {
    /// The keys pinned for `instance`. `Err` starting with
    /// `PEER_KEY_CHANGED` while a change waits for the user.
    fn pinned(&self, instance: &str) -> Result<Option<PublicKeys>, String>;

    /// The relay lists `keys` for `instance`: pinned on first sight and
    /// returned; `Err` (`PEER_KEY_CHANGED`) when they aren't the pinned
    /// ones
    fn learn(&self, instance: &str, keys: &PublicKeys) -> Result<PublicKeys, String>;
}

/// A [`PeerKeyBook`] in memory: pins on first sight, refuses any change
#[derive(Default)]
pub struct MemoryKeyBook {
    pinned: Mutex<HashMap<String, PublicKeys>>,
}

impl PeerKeyBook for MemoryKeyBook {
    fn pinned(&self, instance: &str) -> Result<Option<PublicKeys>, String> {
        Ok(self.lock().get(instance).cloned())
    }

    fn learn(&self, instance: &str, keys: &PublicKeys) -> Result<PublicKeys, String> {
        let mut pinned = self.lock();
        match pinned.get(instance) {
            Some(known) if known != keys => {
                Err(crate::services::network::peer_keys::changed(instance))
            }
            Some(known) => Ok(known.clone()),
            None => {
                pinned.insert(instance.to_string(), keys.clone());
                Ok(keys.clone())
            }
        }
    }
}

impl MemoryKeyBook {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, PublicKeys>> {
        self.pinned.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// What opening an envelope came to, when it didn't fail
#[derive(Debug, PartialEq)]
pub enum Opening<T> {
    Opened(T),
    /// A `(task, from, seq)` already opened: dropped
    Duplicate,
}

/// What an envelope must be, to be opened
struct Expected<'a> {
    task_id: &'a str,
    /// The sender's instance; `None` for a resealed request (any provider
    /// may hand it on; the author is still checked)
    from: Option<&'a str>,
    /// The capability a request or result binds
    capability: &'a str,
}

/// Bounded memory of the `(task, from, seq)` opened
#[derive(Default)]
struct Seen {
    keys: HashSet<(String, String, u64)>,
    order: VecDeque<(String, String, u64)>,
}

impl Seen {
    fn contains(&self, sealed_for: &SealedFor) -> bool {
        self.keys.contains(&Self::key(sealed_for))
    }

    fn insert(&mut self, sealed_for: &SealedFor) {
        let key = Self::key(sealed_for);
        if self.keys.insert(key.clone()) {
            self.order.push_back(key);
            while self.order.len() > SEEN_KEPT {
                if let Some(old) = self.order.pop_front() {
                    self.keys.remove(&old);
                }
            }
        }
    }

    fn key(sealed_for: &SealedFor) -> (String, String, u64) {
        (
            sealed_for.task_id.clone(),
            sealed_for.from.clone(),
            sealed_for.seq,
        )
    }
}

/// Opened requests by the task ID they reached us under
#[derive(Default)]
struct Requests {
    by_task: HashMap<String, Opened>,
    order: VecDeque<String>,
}

impl Requests {
    fn insert(&mut self, task_id: &str, opened: Opened) {
        if self.by_task.insert(task_id.to_string(), opened).is_none() {
            self.order.push_back(task_id.to_string());
            while self.order.len() > REQUESTS_KEPT {
                if let Some(old) = self.order.pop_front() {
                    self.by_task.remove(&old);
                }
            }
        }
    }
}

/// This instance's sealing and opening, with its own keys
pub struct Sealer {
    me: String,
    keys: DeviceKeys,
    /// The next envelope's `seq`: from the clock in microseconds, so it
    /// keeps increasing across restarts
    next_seq: AtomicU64,
    seen: Mutex<Seen>,
    requests: Mutex<Requests>,
}

impl Sealer {
    pub fn new(me: impl Into<String>, keys: DeviceKeys) -> Self {
        let start = u64::try_from(chrono::Utc::now().timestamp_micros()).unwrap_or_default();
        Self {
            me: me.into(),
            keys,
            next_seq: AtomicU64::new(start),
            seen: Mutex::default(),
            requests: Mutex::default(),
        }
    }

    pub fn public_keys(&self) -> &PublicKeys {
        self.keys.public_keys()
    }

    /// The instance it seals as
    pub fn me(&self) -> &str {
        &self.me
    }

    fn sealed_for(
        &self,
        task_id: &str,
        to: &str,
        purpose: wire::SealPurpose,
        capability: &str,
    ) -> SealedFor {
        SealedFor {
            task_id: task_id.to_string(),
            seq: self.next_seq.fetch_add(1, Ordering::Relaxed),
            from: self.me.clone(),
            to: to.to_string(),
            purpose,
            capability: capability.to_string(),
        }
    }

    /// `content` sealed for `to`, signed by us
    fn seal<C: SealedContent>(
        &self,
        task_id: &str,
        (to, to_keys): (&str, &PublicKeys),
        capability: &str,
        content: &C,
    ) -> Result<Envelope, String> {
        let bytes = content.to_bytes();
        let encoded = (bytes.len() + SEAL_OVERHEAD).div_ceil(3) * 4;
        if encoded + ENC_CHARS > MAX_ENVELOPE_BYTES {
            return Err(too_large(bytes.len()));
        }
        let sealed_for = self.sealed_for(task_id, to, C::PURPOSE, capability);
        let envelope = nandi_seal::seal(&self.keys, to_keys, sealed_for, &bytes)
            .map_err(|e| format!("Couldn't seal for {to}: {e}"))?;
        if !envelope.is_within_limit() {
            return Err(too_large(bytes.len()));
        }
        Ok(envelope)
    }

    /// `text` (when there is any) sealed for `to` as a note on `task_id`
    /// (a duty's ID for a duty's error)
    pub fn note(
        &self,
        task_id: &str,
        to: (&str, &PublicKeys),
        text: Option<String>,
    ) -> Result<Option<Envelope>, String> {
        text.filter(|text| !text.is_empty())
            .map(|text| self.seal(task_id, to, LIFECYCLE_CAPABILITY, &NoteContent { text }))
            .transpose()
    }

    /// Opens `envelope` from `author`, checking it's what `expected` says;
    /// the content and what `nandi-seal` opened
    fn open<C: SealedContent>(
        &self,
        envelope: &Envelope,
        expected: &Expected<'_>,
        author: (&str, &PublicKeys),
    ) -> Result<Opening<(C, Opened)>, String> {
        let task_id = expected.task_id;
        let refuse = |why: &str| refused(author.0, task_id, why);
        if !envelope.is_within_limit() {
            return Err(refuse(&format!("it's over {MAX_ENVELOPE_BYTES} bytes")));
        }
        let sealed_for = &envelope.sealed_for;
        if sealed_for.to != self.me {
            return Err(refuse("it was sealed for someone else"));
        }
        if sealed_for.task_id != task_id {
            return Err(refuse("it was sealed for another task"));
        }
        if sealed_for.purpose != C::PURPOSE {
            return Err(refuse(&format!(
                "it holds a {}, not a {}",
                sealed_for.purpose.as_str(),
                C::PURPOSE.as_str()
            )));
        }
        if sealed_for.capability != expected.capability {
            return Err(refuse("it was sealed for another capability"));
        }
        if expected.from.is_some_and(|from| sealed_for.from != from) {
            return Err(refuse("it was sealed by someone else"));
        }
        if self.lock_seen().contains(sealed_for) {
            return Ok(Opening::Duplicate);
        }
        let opened =
            nandi_seal::open(&self.keys, envelope, author.1).map_err(|e| refuse(&e.to_string()))?;
        let content = C::from_bytes(&opened.content).map_err(|e| refuse(&e))?;
        self.lock_seen().insert(sealed_for);
        Ok(Opening::Opened((content, opened)))
    }

    /// The note in `envelope` (if any) on `task_id`, sealed for us by
    /// `from`
    pub fn open_note(
        &self,
        envelope: Option<&Envelope>,
        task_id: &str,
        from: (&str, &PublicKeys),
    ) -> Result<Opening<Option<String>>, String> {
        let Some(envelope) = envelope else {
            return Ok(Opening::Opened(None));
        };
        let expected = Expected {
            task_id,
            from: Some(from.0),
            capability: LIFECYCLE_CAPABILITY,
        };
        Ok(match self.open::<NoteContent>(envelope, &expected, from)? {
            Opening::Opened((note, _)) => Opening::Opened(Some(note.text)),
            Opening::Duplicate => Opening::Duplicate,
        })
    }

    fn lock_seen(&self) -> std::sync::MutexGuard<'_, Seen> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn lock_requests(&self) -> std::sync::MutexGuard<'_, Requests> {
        self.requests.lock().unwrap_or_else(|e| e.into_inner())
    }

    // -- Tasks --------------------------------------------------------------

    /// The input of `task`, sealed for its provider
    pub fn seal_task(
        &self,
        task: &Task,
        provider: (&str, &PublicKeys),
    ) -> Result<Envelope, String> {
        self.seal(
            &task.id,
            provider,
            &task.capability,
            &RequestContent {
                input: task.input.clone(),
            },
        )
    }

    /// The task in `sealed`, which the relay says the requester (whose
    /// keys are `requester`) sent us
    pub fn open_task(
        &self,
        sealed: SealedTask,
        requester: &PublicKeys,
    ) -> Result<Opening<Task>, String> {
        let author = sealed.requester.instance_id.clone();
        // Sealed by the requester, or resealed by a provider it was
        // reassigned from: either way the requester's signature is checked
        let expected = Expected {
            task_id: &sealed.id,
            from: None,
            capability: &sealed.capability,
        };
        let (content, opened) =
            match self.open::<RequestContent>(&sealed.envelope, &expected, (&author, requester))? {
                Opening::Opened(both) => both,
                Opening::Duplicate => return Ok(Opening::Duplicate),
            };
        self.lock_requests().insert(&sealed.id, opened);
        Ok(Opening::Opened(Task {
            id: sealed.id,
            requester: sealed.requester,
            target: sealed.target,
            capability: sealed.capability,
            input: content.input,
            created_at: sealed.created_at,
            deadline: sealed.deadline,
        }))
    }

    /// Whether we hold the opened request of `task_id`, to reseal it
    pub fn holds_request(&self, task_id: &str) -> bool {
        self.lock_requests().by_task.contains_key(task_id)
    }

    /// The request of `task_id` resealed for `to`, the requester's
    /// signature kept; `None` when we no longer hold it
    pub fn reseal_request(
        &self,
        task_id: &str,
        to: (&str, &PublicKeys),
    ) -> Result<Option<Envelope>, String> {
        let requests = self.lock_requests();
        let Some(opened) = requests.by_task.get(task_id) else {
            return Ok(None);
        };
        let before = &opened.sealed_for;
        let sealed_for = self.sealed_for(&before.task_id, to.0, before.purpose, &before.capability);
        nandi_seal::reseal(&self.keys, opened, to.1, sealed_for)
            .map(Some)
            .map_err(|e| format!("Couldn't reseal the request for {}: {e}", to.0))
    }

    // -- Results ------------------------------------------------------------

    /// `result`, its output and error sealed for the requester
    pub fn seal_result(
        &self,
        result: &TaskResult,
        requester: (&str, &PublicKeys),
    ) -> Result<ResultReport, String> {
        let envelope = self.seal(
            &result.task_id,
            requester,
            &result.capability,
            &ResultContent {
                output: result.output.clone(),
                error: result.error.clone(),
            },
        )?;
        Ok(ResultReport {
            task_id: result.task_id.clone(),
            provider_agent: result.provider_agent.clone(),
            requester_agent: result.requester_agent.clone(),
            capability: result.capability.clone(),
            status: result.status,
            duration_ms: result.duration_ms,
            tokens_in: result.tokens_in,
            tokens_out: result.tokens_out,
            provider_owner: result.provider_owner.clone(),
            requester_owner: result.requester_owner.clone(),
            started_at: result.started_at,
            ended_at: result.ended_at,
            envelope,
        })
    }

    /// The result in `report` from `provider` (the input stays ours: it's
    /// left `null`)
    pub fn open_result(
        &self,
        report: ResultReport,
        provider: (&str, &PublicKeys),
    ) -> Result<Opening<TaskResult>, String> {
        let expected = Expected {
            task_id: &report.task_id,
            from: Some(provider.0),
            capability: &report.capability,
        };
        let content = match self.open::<ResultContent>(&report.envelope, &expected, provider)? {
            Opening::Opened((content, _)) => content,
            Opening::Duplicate => return Ok(Opening::Duplicate),
        };
        Ok(Opening::Opened(TaskResult {
            task_id: report.task_id,
            provider_agent: report.provider_agent,
            output: content.output,
            status: report.status,
            duration_ms: report.duration_ms,
            tokens_in: report.tokens_in,
            tokens_out: report.tokens_out,
            error: content.error,
            direction: DelegationDirection::Outbound,
            capability: report.capability,
            requester_agent: report.requester_agent,
            provider_owner: report.provider_owner,
            requester_owner: report.requester_owner,
            input: serde_json::Value::Null,
            started_at: report.started_at,
            ended_at: report.ended_at,
        }))
    }

    // -- Peer messages ------------------------------------------------------

    /// Whether `body` carries content, so it needs the target's keys
    pub fn has_content(body: &PeerMessage) -> bool {
        let text = |t: &Option<String>| t.as_ref().is_some_and(|t| !t.is_empty());
        match body {
            PeerMessage::AcknowledgeTask { note, .. }
            | PeerMessage::ProgressUpdate { note, .. } => text(note),
            PeerMessage::RejectTask { reason, .. } => !reason.is_empty(),
            PeerMessage::CancelTask { reason, .. } => text(reason),
            PeerMessage::ClarificationRequest(_) | PeerMessage::ClarificationResponse(_) => true,
            PeerMessage::StatusQuery { .. }
            | PeerMessage::StatusResponse { .. }
            | PeerMessage::TaskDelivered { .. }
            | PeerMessage::ClarificationTimeout { .. } => false,
        }
    }

    /// `body` for `target`, its content sealed with `target_keys` (needed
    /// only when [`Self::has_content`])
    pub fn seal_peer(
        &self,
        target: &str,
        body: PeerMessage,
        target_keys: Option<&PublicKeys>,
    ) -> Result<wire::PeerMessage, String> {
        let task_id = body.task_id().to_string();
        let to = || -> Result<(&str, &PublicKeys), String> {
            target_keys
                .map(|keys| (target, keys))
                .ok_or_else(|| missing(target))
        };
        let note = |text: Option<String>| -> Result<Option<Envelope>, String> {
            match text.filter(|t| !t.is_empty()) {
                Some(text) => self.note(&task_id, to()?, Some(text)),
                None => Ok(None),
            }
        };
        Ok(match body {
            PeerMessage::AcknowledgeTask {
                task_id,
                eta_seconds,
                note: text,
            } => wire::PeerMessage::AcknowledgeTask {
                envelope: note(text)?,
                task_id,
                eta_seconds,
            },
            PeerMessage::RejectTask {
                task_id,
                reason,
                reason_kind,
            } => wire::PeerMessage::RejectTask {
                envelope: note(Some(reason))?,
                task_id,
                reason_kind,
            },
            PeerMessage::ProgressUpdate {
                task_id,
                percent,
                note: text,
                reason_kind,
                queue_position,
            } => wire::PeerMessage::ProgressUpdate {
                envelope: note(text)?,
                task_id,
                percent,
                reason_kind,
                queue_position,
            },
            PeerMessage::StatusQuery { task_id } => wire::PeerMessage::StatusQuery { task_id },
            PeerMessage::StatusResponse {
                task_id,
                state,
                eta_seconds,
                elapsed_seconds,
            } => wire::PeerMessage::StatusResponse {
                task_id,
                state,
                eta_seconds,
                elapsed_seconds,
            },
            PeerMessage::CancelTask { task_id, reason } => wire::PeerMessage::CancelTask {
                envelope: note(reason)?,
                task_id,
            },
            PeerMessage::TaskDelivered { task_id } => wire::PeerMessage::TaskDelivered { task_id },
            PeerMessage::ClarificationRequest(request) => wire::PeerMessage::ClarificationRequest {
                envelope: self.seal(
                    &request.task_id,
                    to()?,
                    LIFECYCLE_CAPABILITY,
                    &ClarificationContent {
                        questions: request.questions,
                    },
                )?,
                task_id: request.task_id,
                round: request.round,
            },
            PeerMessage::ClarificationResponse(response) => {
                wire::PeerMessage::ClarificationResponse {
                    envelope: self.seal(
                        &response.task_id,
                        to()?,
                        LIFECYCLE_CAPABILITY,
                        &AnswerContent {
                            answers: response.answers,
                        },
                    )?,
                    task_id: response.task_id,
                    round: response.round,
                }
            }
            PeerMessage::ClarificationTimeout { task_id } => {
                wire::PeerMessage::ClarificationTimeout { task_id }
            }
        })
    }

    /// `body` from `from` (whose keys are `from_keys`, needed only when it
    /// carries an envelope), opened
    pub fn open_peer(
        &self,
        from: &str,
        body: wire::PeerMessage,
        from_keys: Option<&PublicKeys>,
    ) -> Result<Opening<PeerMessage>, String> {
        let task_id = body.task_id().to_string();
        let keys = || -> Result<(&str, &PublicKeys), String> {
            from_keys
                .map(|keys| (from, keys))
                .ok_or_else(|| refused(from, &task_id, "we don't have their keys"))
        };
        let note = |envelope: Option<Envelope>| -> Result<Opening<Option<String>>, String> {
            match envelope {
                Some(envelope) => self.open_note(Some(&envelope), &task_id, keys()?),
                None => Ok(Opening::Opened(None)),
            }
        };
        let lifecycle = Expected {
            task_id: &task_id,
            from: Some(from),
            capability: LIFECYCLE_CAPABILITY,
        };
        macro_rules! opened {
            ($opening:expr) => {
                match $opening {
                    Opening::Opened(value) => value,
                    Opening::Duplicate => return Ok(Opening::Duplicate),
                }
            };
        }
        Ok(Opening::Opened(match body {
            wire::PeerMessage::AcknowledgeTask {
                task_id,
                eta_seconds,
                envelope,
            } => PeerMessage::AcknowledgeTask {
                note: opened!(note(envelope)?),
                task_id,
                eta_seconds,
            },
            wire::PeerMessage::RejectTask {
                task_id,
                reason_kind,
                envelope,
            } => PeerMessage::RejectTask {
                reason: opened!(note(envelope)?).unwrap_or_default(),
                task_id,
                reason_kind,
            },
            wire::PeerMessage::ProgressUpdate {
                task_id,
                percent,
                reason_kind,
                queue_position,
                envelope,
            } => PeerMessage::ProgressUpdate {
                note: opened!(note(envelope)?),
                task_id,
                percent,
                reason_kind,
                queue_position,
            },
            wire::PeerMessage::StatusQuery { task_id } => PeerMessage::StatusQuery { task_id },
            wire::PeerMessage::StatusResponse {
                task_id,
                state,
                eta_seconds,
                elapsed_seconds,
            } => PeerMessage::StatusResponse {
                task_id,
                state,
                eta_seconds,
                elapsed_seconds,
            },
            wire::PeerMessage::CancelTask { task_id, envelope } => PeerMessage::CancelTask {
                reason: opened!(note(envelope)?),
                task_id,
            },
            wire::PeerMessage::TaskDelivered { task_id } => PeerMessage::TaskDelivered { task_id },
            wire::PeerMessage::ClarificationRequest {
                task_id,
                round,
                envelope,
            } => {
                let (content, _) =
                    opened!(self.open::<ClarificationContent>(&envelope, &lifecycle, keys()?)?);
                PeerMessage::ClarificationRequest(ClarificationRequest {
                    task_id,
                    round,
                    questions: content.questions,
                })
            }
            wire::PeerMessage::ClarificationResponse {
                task_id,
                round,
                envelope,
            } => {
                let (content, _) =
                    opened!(self.open::<AnswerContent>(&envelope, &lifecycle, keys()?)?);
                PeerMessage::ClarificationResponse(ClarificationResponse {
                    task_id,
                    round,
                    answers: content.answers,
                })
            }
            wire::PeerMessage::ClarificationTimeout { task_id } => {
                PeerMessage::ClarificationTimeout { task_id }
            }
        }))
    }
}

/// The refusal of something from `from` about `task_id` that didn't open
pub fn refused(from: &str, task_id: &str, why: &str) -> String {
    format!(
        "{SEAL_REFUSED}: a message from {from} about {task_id} was refused because {why}. Nothing in it was used"
    )
}

/// The refusal to send to `instance`, who has no keys at the relay
pub fn missing(instance: &str) -> String {
    format!(
        "{PEER_KEY_MISSING}: {instance} hasn't published keys to the relay, so nothing can be sealed for them. They need to update Nandi and connect"
    )
}

fn too_large(bytes: usize) -> String {
    format!(
        "{ENVELOPE_TOO_LARGE}: this is {bytes} bytes, over the {MAX_ENVELOPE_BYTES}-byte limit for a message to another person. Share files as links"
    )
}

#[cfg(test)]
#[path = "sealing_tests.rs"]
mod tests;
