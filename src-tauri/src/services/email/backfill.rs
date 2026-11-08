//! The background read of older mail, so My notes is built from the whole
//! history, not only the first 14 days.
//!
//! - It starts where the first read stopped and walks older, newest
//!   first: Inbox and Sent, one batch of at most [`BATCH`] emails per pass,
//!   every [`BACKFILL_EVERY`]. It only builds the profile, so it reads less
//!   than new mail does ([`MailFilter::keep_older`]): nothing the user never
//!   opened, no system updates, no phishing.
//! - Its position is its own (`local_settings` `email.<provider>.backfill`:
//!   the oldest time reached); it never touches the change cursor.
//! - A pass runs only when no email check and no other Chief of Staff run
//!   is going, so it never competes with new mail.
//! - It stops at the start of the mailbox or the chosen range
//!   ([`BackfillRange`]); a 429 or an outage just waits for the next pass.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use super::batch::{Batch, MAX_BATCH};
use super::filter::MailFilter;
use super::mail::Identity;
use super::mailbox::Mailbox;
use super::settings::{BackfillRange, BackfillState};
use super::{
    email_settings, emit_changed, granted, load_setting, save_setting, status, EmailStatus,
    MailProvider, READ_WINDOW_DAYS,
};
use crate::db;
use crate::model::Timestamp;
use crate::services::builtin_agents::CHIEF_OF_STAFF;
use crate::services::AppState;

/// How often a batch of older mail goes to the Chief of Staff
pub const BACKFILL_EVERY: Duration = Duration::from_secs(10 * 60);
/// Emails asked for per batch
const BATCH: u32 = MAX_BATCH as u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BackfillPhase {
    /// Not chosen, or not connected
    Off,
    /// Starts after the first read
    Waiting,
    Reading,
    Paused,
    /// Read back to the start of the mailbox or the range, or stopped
    Done,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BackfillStatus {
    pub phase: BackfillPhase,
    /// Everything newer than this has been read
    pub back_to: Option<Timestamp>,
    /// Emails handed to the Chief of Staff so far
    pub read: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum BackfillAction {
    Pause,
    Resume,
    /// Ends it for this mailbox (Settings can start it again)
    Stop,
    /// From the top again
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BackfillEstimate {
    pub provider: MailProvider,
    /// About how many emails the range holds (bulk not left out yet)
    pub emails: u32,
    /// About how many Chief of Staff runs that is
    pub runs: u32,
}

pub(super) async fn load(state: &AppState, provider: MailProvider) -> BackfillState {
    load_setting(state, provider.backfill_key()).await
}

async fn save(state: &AppState, provider: MailProvider, backfill: &BackfillState) {
    let value = serde_json::to_value(backfill).unwrap_or_default();
    if let Err(e) = save_setting(state, provider.backfill_key(), value).await {
        log::warn!(
            "Couldn't save the {} older-mail position: {e}",
            provider.name()
        );
    }
}

/// What the mailbox card shows
pub(super) fn status_of(
    range: BackfillRange,
    connected: bool,
    first_read_done: bool,
    backfill: &BackfillState,
) -> BackfillStatus {
    let phase = if !connected || range == BackfillRange::Off {
        BackfillPhase::Off
    } else if backfill.done {
        BackfillPhase::Done
    } else if backfill.paused {
        BackfillPhase::Paused
    } else if !first_read_done {
        BackfillPhase::Waiting
    } else {
        BackfillPhase::Reading
    };
    BackfillStatus {
        phase,
        back_to: backfill.before.map(|s| (s * 1000) as Timestamp),
        read: backfill.read,
    }
}

/// Pause, Resume, Stop or Restart one mailbox's background read
pub async fn set_backfill(
    state: &AppState,
    provider: MailProvider,
    action: BackfillAction,
) -> Result<EmailStatus, String> {
    if email_settings(state).account(provider).is_none() {
        return Err(format!("{} isn't connected", provider.name()));
    }
    let mut backfill = load(state, provider).await;
    match action {
        BackfillAction::Pause => backfill.paused = true,
        BackfillAction::Resume => backfill.paused = false,
        BackfillAction::Stop => backfill.done = true,
        BackfillAction::Restart => backfill = BackfillState::default(),
    }
    save(state, provider, &backfill).await;
    emit_changed(state);
    Ok(status(state).await)
}

fn unix_now() -> i64 {
    db::now_ms() / 1000
}

/// Sets where the background read starts: the oldest point the first read
/// covered (its list is capped, and a batch can leave the oldest out)
pub(super) async fn begin(state: &AppState, provider: MailProvider, before: i64) {
    let mut backfill = load(state, provider).await;
    if backfill.before.is_none() {
        backfill.before = Some(before);
        save(state, provider, &backfill).await;
    }
}

/// Where a mailbox's background read starts: where it got to, else where
/// the first read stopped
fn start_of(backfill: &BackfillState, now: i64) -> i64 {
    backfill
        .before
        .unwrap_or(now - READ_WINDOW_DAYS * 24 * 60 * 60)
}

/// About how much older mail `range` covers in each connected mailbox
pub async fn estimate(state: &AppState, range: BackfillRange) -> Vec<BackfillEstimate> {
    if range == BackfillRange::Off {
        return Vec::new();
    }
    let setup = state.email().setup();
    let now = unix_now();
    let mut out = Vec::new();
    for account in email_settings(state).accounts {
        let backfill = load(state, account.provider).await;
        let mailbox = Mailbox::new(account.provider, &setup, state.secrets());
        match mailbox
            .count(start_of(&backfill, now), range.floor(now))
            .await
        {
            Ok(count) => {
                let emails = u32::try_from(count).unwrap_or(u32::MAX);
                out.push(BackfillEstimate {
                    provider: account.provider,
                    emails,
                    runs: emails.div_ceil(BATCH),
                });
            }
            Err(e) => log::debug!(
                "No older-mail estimate for {}: {e}",
                account.provider.name()
            ),
        }
    }
    out
}

/// Whether the Chief of Staff is busy (a run going or waiting)
async fn chief_of_staff_busy(state: &AppState) -> bool {
    state
        .db()
        .call(|conn| {
            Ok(db::active_agent_run(conn, CHIEF_OF_STAFF)?.is_some()
                || !db::list_queue(conn, CHIEF_OF_STAFF)?.is_empty())
        })
        .await
        .unwrap_or(true)
}

/// One pass: the next batch of older mail from the first mailbox that has
/// some, as one Chief of Staff run. Returns its run ID.
pub async fn pass(state: &AppState) -> Option<String> {
    let settings = email_settings(state);
    if settings.backfill == BackfillRange::Off || state.email().is_checking() {
        return None;
    }
    if chief_of_staff_busy(state).await {
        log::debug!("Older mail waits: the Chief of Staff is busy");
        return None;
    }
    let setup = state.email().setup();
    let now = unix_now();
    let floor = settings.backfill.floor(now);
    let me: Vec<Identity> = settings
        .accounts
        .iter()
        .map(|a| Identity {
            name: a.name.clone(),
            email: a.email.clone(),
        })
        .collect();
    let addresses: Vec<String> = me.iter().map(|i| i.email.clone()).collect();
    let filter = MailFilter::new(&addresses, &settings.watch);
    for account in granted(state, CHIEF_OF_STAFF).iter().filter(|a| !a.paused) {
        let provider = account.provider;
        let mut backfill = load(state, provider).await;
        let cursor: Value = load_setting(state, provider.cursor_key()).await;
        // After the first read only
        if backfill.done || backfill.paused || cursor.is_null() {
            continue;
        }
        let before = start_of(&backfill, now);
        if floor.is_some_and(|floor| before <= floor) {
            backfill.done = true;
            save(state, provider, &backfill).await;
            emit_changed(state);
            continue;
        }
        let mailbox = Mailbox::new(provider, &setup, state.secrets());
        let mut headers = match mailbox.older(before, floor, BATCH).await {
            Ok(headers) => headers,
            Err(e) => {
                // Throttled or offline: the next pass tries again
                log::info!("Reading older {} mail waits: {e}", provider.name());
                return None;
            }
        };
        // Strictly older than where it got to: a provider that rounds the
        // boundary would hand the same email over every pass
        let listed = headers.len();
        headers.retain(|h| h.at < before);
        let Some(oldest) = headers.iter().map(|h| h.at).min() else {
            if listed > 0 {
                // Only the boundary came back: step past it
                backfill.before = Some(before - 1);
                save(state, provider, &backfill).await;
                return None;
            }
            log::info!("Read all older {} mail", provider.name());
            backfill.done = true;
            save(state, provider, &backfill).await;
            emit_changed(state);
            continue;
        };
        // Only what tells who the user is: not what they never opened,
        // machines' updates or phishing
        headers.retain(|h| filter.keep_older(h));
        mailbox.mark_bulk(&mut headers).await;
        headers.retain(|h| filter.keep_older(h));
        if headers.is_empty() {
            // Nothing worth reading in this stretch: move on without a run
            backfill.before = Some(oldest);
            save(state, provider, &backfill).await;
            emit_changed(state);
            return None;
        }
        let text = Batch {
            me: &me,
            headers: &headers,
            omitted: 0,
            first_read: false,
            window_days: READ_WINDOW_DAYS,
            backfill: true,
        }
        .prompt();
        let title = format!("Email: older mail, {} emails", headers.len());
        return match state.email().sink().submit(state, title, text).await {
            Ok(run_id) => {
                backfill.before = Some(oldest);
                backfill.read = backfill
                    .read
                    .saturating_add(u32::try_from(headers.len()).unwrap_or(u32::MAX));
                save(state, provider, &backfill).await;
                emit_changed(state);
                Some(run_id)
            }
            Err(e) => {
                log::warn!("The Chief of Staff couldn't take older mail: {e}");
                None
            }
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_card_says_where_the_read_is() {
        let fresh = BackfillState::default();
        let range = BackfillRange::Everything;
        assert_eq!(
            status_of(range, false, true, &fresh).phase,
            BackfillPhase::Off
        );
        assert_eq!(
            status_of(BackfillRange::Off, true, true, &fresh).phase,
            BackfillPhase::Off
        );
        assert_eq!(
            status_of(range, true, false, &fresh).phase,
            BackfillPhase::Waiting
        );
        let reading = BackfillState {
            before: Some(1_700_000_000),
            read: 120,
            ..BackfillState::default()
        };
        let status = status_of(range, true, true, &reading);
        assert_eq!(status.phase, BackfillPhase::Reading);
        assert_eq!(status.back_to, Some(1_700_000_000_000.0));
        assert_eq!(status.read, 120);
        let paused = BackfillState {
            paused: true,
            ..reading.clone()
        };
        assert_eq!(
            status_of(range, true, true, &paused).phase,
            BackfillPhase::Paused
        );
        let done = BackfillState {
            done: true,
            ..reading
        };
        assert_eq!(
            status_of(range, true, true, &done).phase,
            BackfillPhase::Done
        );
    }

    #[test]
    fn ranges_have_floors() {
        let now = 1_800_000_000;
        assert_eq!(BackfillRange::Everything.floor(now), None);
        assert_eq!(BackfillRange::Year.floor(now), Some(now - 365 * 86_400));
        assert_eq!(
            BackfillRange::ThreeMonths.floor(now),
            Some(now - 91 * 86_400)
        );
    }
}
