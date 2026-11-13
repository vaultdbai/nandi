//! Audit log commands (Tauri adapter for `audit::AuditStore`).

use crate::audit::AuditEntry;

super::commands! {
    /// `log_kind`: `llm` | `egress` | `triggers` | `pilots`; newest first
    fn list_audit_log(state, log_kind: String, limit: u32, offset: u32) -> Vec<AuditEntry> => state.audit_store().list(&log_kind, limit, offset).await;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::services::AppState;

    #[tokio::test]
    async fn list_audit_log_checks_the_kind() {
        let state = AppState::for_tests(vec![]);
        let args = json!({ "logKind": "bogus", "limit": 10, "offset": 0 });
        let result = crate::rpc::call(state.clone(), "list_audit_log", args)
            .await
            .unwrap();
        state.shutdown();
        assert!(result.unwrap_err().contains("Unknown audit log"));
    }
}
