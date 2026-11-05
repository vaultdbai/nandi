//! Email commands (Tauri adapter for `services::email`).

use tauri::State;

use crate::services::auth::oauth::DeviceFlowState;
use crate::services::email::{
    self, EmailSettingsInput, EmailSignInPoll, EmailStatus, MailProvider,
};
use crate::services::AppState;

/// The mailboxes, what this build can connect, and when the next check is
#[tauri::command]
#[specta::specta]
pub async fn get_email_status(state: State<'_, AppState>) -> Result<EmailStatus, String> {
    Ok(email::status(&state).await)
}

/// Starts connecting a mailbox: open `verificationUri` (PKCE), or show
/// `userCode` (device flow)
#[tauri::command]
#[specta::specta]
pub async fn start_email_sign_in(
    state: State<'_, AppState>,
    provider: MailProvider,
) -> Result<DeviceFlowState, String> {
    email::start_sign_in(&state, provider).await
}

/// Checks the sign-in once; `connected` once the mailbox is set up
#[tauri::command]
#[specta::specta]
pub async fn poll_email_sign_in(
    state: State<'_, AppState>,
    provider: MailProvider,
) -> Result<EmailSignInPoll, String> {
    Ok(email::poll_sign_in(&state, provider).await)
}

/// Checks every connected mailbox now, in the background
#[tauri::command]
#[specta::specta]
pub async fn check_email_now(state: State<'_, AppState>) -> Result<EmailStatus, String> {
    Ok(email::check_now(&state).await)
}

#[tauri::command]
#[specta::specta]
pub async fn set_email_paused(
    state: State<'_, AppState>,
    provider: MailProvider,
    paused: bool,
) -> Result<EmailStatus, String> {
    email::set_paused(&state, provider, paused).await
}

/// Signs out: deletes the sign-in, the cursor and the Chief of Staff's access
#[tauri::command]
#[specta::specta]
pub async fn disconnect_email(
    state: State<'_, AppState>,
    provider: MailProvider,
) -> Result<EmailStatus, String> {
    email::disconnect(&state, provider).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_email_settings(
    state: State<'_, AppState>,
    settings: EmailSettingsInput,
) -> Result<EmailStatus, String> {
    email::set_settings(&state, settings).await
}
