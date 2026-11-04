//! Audit log commands (Tauri adapter for `audit::AuditStore`).

use tauri::State;

pub use crate::audit::AuditEntry;
use crate::services::AppState;

/// `log_kind`: `llm` | `egress` | `triggers` | `pilots`; newest first
#[tauri::command]
#[specta::specta]
pub async fn list_audit_log(
    state: State<'_, AppState>,
    log_kind: String,
    limit: u32,
    offset: u32,
) -> Result<Vec<AuditEntry>, String> {
    state.audit_store().list(&log_kind, limit, offset).await
}
