//! Email commands (`services::email`): mailboxes, sign-in, checks and the backfill.

use crate::services::auth::oauth::DeviceFlowState;
use crate::services::email::{
    self, BackfillAction, BackfillEstimate, BackfillRange, EmailSettingsInput, EmailSignInPoll,
    EmailStatus, MailProvider, MailboxId,
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

    /// Reads the last few days of mail again, in the background (the
    /// user's resync); the change cursors stay where they are
    fn read_email_again(state) -> EmailStatus => Ok(email::read_again(&state).await);

    /// Pauses or resumes reading one mailbox
    fn set_email_paused(state, mailbox: MailboxId, paused: bool) -> EmailStatus => email::set_paused(&state, mailbox, paused).await;

    /// Signs one mailbox out: deletes its sign-in and cursor, and Nandi's
    /// access with the provider's last mailbox
    fn disconnect_email(state, mailbox: MailboxId) -> EmailStatus => email::disconnect(&state, mailbox).await;

    fn set_email_settings(state, settings: EmailSettingsInput) -> EmailStatus => email::set_settings(&state, settings).await;

    /// Pause, Resume, Stop or Restart one mailbox's background read of older mail
    fn set_email_backfill(state, mailbox: MailboxId, action: BackfillAction) -> EmailStatus => email::backfill::set_backfill(&state, mailbox, action).await;

    /// About how much older mail `range` covers, per connected mailbox
    fn estimate_email_backfill(state, range: BackfillRange) -> Vec<BackfillEstimate> => Ok(email::backfill::estimate(&state, range).await);
}
