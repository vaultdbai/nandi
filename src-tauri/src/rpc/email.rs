//! Email commands (`services::email`): mailboxes, sign-in, checks and the backfill.

use crate::services::auth::oauth::DeviceFlowState;
use crate::services::email::{
    self, BackfillAction, BackfillEstimate, BackfillRange, EmailSettingsInput, EmailSignInPoll,
    EmailStatus, MailProvider,
};

super::commands! {
    /// The mailboxes, what this build can connect, and when the next check is
    fn get_email_status(state) -> EmailStatus => Ok(email::status(&state).await);

    /// Starts connecting a mailbox: open `verificationUri` (PKCE), or show
    /// `userCode` (device flow)
    fn start_email_sign_in(state, provider: MailProvider) -> DeviceFlowState => email::start_sign_in(&state, provider).await;

    /// Checks the sign-in once; `connected` once the mailbox is set up
    fn poll_email_sign_in(state, provider: MailProvider) -> EmailSignInPoll => Ok(email::poll_sign_in(&state, provider).await);

    /// Checks every connected mailbox now, in the background
    fn check_email_now(state) -> EmailStatus => Ok(email::check_now(&state).await);

    fn set_email_paused(state, provider: MailProvider, paused: bool) -> EmailStatus => email::set_paused(&state, provider, paused).await;

    /// Signs out: deletes the sign-in, the cursor and the Chief of Staff's access
    fn disconnect_email(state, provider: MailProvider) -> EmailStatus => email::disconnect(&state, provider).await;

    fn set_email_settings(state, settings: EmailSettingsInput) -> EmailStatus => email::set_settings(&state, settings).await;

    /// Pause, Resume, Stop or Restart one mailbox's background read of older mail
    fn set_email_backfill(state, provider: MailProvider, action: BackfillAction) -> EmailStatus => email::backfill::set_backfill(&state, provider, action).await;

    /// About how much older mail `range` covers, per connected mailbox
    fn estimate_email_backfill(state, range: BackfillRange) -> Vec<BackfillEstimate> => Ok(email::backfill::estimate(&state, range).await);
}
