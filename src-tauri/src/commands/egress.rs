//! Egress commands: proxy status and AWS credentials.

use tauri::State;

use crate::services::egress::{self, EgressStatus};
use crate::services::AppState;

#[tauri::command]
#[specta::specta]
pub async fn get_egress_status(state: State<'_, AppState>) -> Result<EgressStatus, String> {
    egress::get_egress_status(&state).await
}

/// Stores the AWS key in the keychain; empty values remove it.
#[tauri::command]
#[specta::specta]
pub async fn set_aws_credentials(
    state: State<'_, AppState>,
    access_key: String,
    secret: String,
) -> Result<(), String> {
    egress::set_aws_credentials(&state, access_key, secret).await
}

#[tauri::command]
#[specta::specta]
pub async fn set_aws_region(state: State<'_, AppState>, region: String) -> Result<(), String> {
    egress::set_aws_region(&state, region).await
}
