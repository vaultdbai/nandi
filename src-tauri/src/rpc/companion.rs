//! Companion commands that read or save state (`services::companion`). The
//! window itself (showing, placing and pinning it) is desktop-only:
//! `desktop::companion`.

use crate::services::chat::SendMessageResult;
use crate::services::companion::{self, CompanionSnapshot};

super::commands! {
    /// `day_start`: local midnight, epoch milliseconds
    fn get_companion_snapshot(state, day_start: f64) -> CompanionSnapshot => companion::get_companion_snapshot(&state, day_start).await;

    /// Saves the "keep on top" preference only. The desktop's
    /// `set_companion_pinned` also applies it to the window.
    fn save_companion_pinned(state, pinned: bool) -> () => companion::set_pinned(&state, pinned).await;

    /// Start Nandi with only the bison (the main window hidden until opened
    /// from the companion)
    fn set_companion_start_bison_only(state, on: bool) -> () => companion::set_start_bison_only(&state, on).await;

    /// A companion chat turn: Studio (grounded by its tools), scoped to a
    /// Request when `scope_request_id` is set; "@Agent …" asks that agent
    fn send_companion_message(state, text: String, conversation_id: Option<String>, scope_request_id: Option<String>) -> SendMessageResult => companion::send_companion_message(&state, &text, conversation_id.as_deref(), scope_request_id.as_deref()).await;

    /// "Notify me": OS notifications for approvals and questions
    fn set_companion_notify(state, notify: bool) -> () => companion::set_notify(&state, notify).await;

    /// "Notify me when requests come back"
    fn set_companion_notify_results(state, notify: bool) -> () => companion::set_notify_results(&state, notify).await;
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Value};

    use crate::services::AppState;

    async fn call(state: &AppState, name: &str, args: Value) -> Result<Value, String> {
        crate::rpc::call(state.clone(), name, args).await.unwrap()
    }

    fn message(text: &str) -> Value {
        json!({ "text": text, "conversationId": null, "scopeRequestId": null })
    }

    #[tokio::test]
    async fn snapshot_pin_notify_and_messages() {
        let state = AppState::for_tests(vec![]);

        let pinned = call(&state, "save_companion_pinned", json!({ "pinned": true }));
        pinned.await.unwrap();
        let snapshot = call(&state, "get_companion_snapshot", json!({ "dayStart": 0 }));
        let snapshot = snapshot.await.unwrap();
        assert_eq!(snapshot["pinned"], true);
        assert_eq!(snapshot["health"], "healthy");

        let blank = call(&state, "send_companion_message", message(" "));
        assert!(blank.await.is_err());
        let sent = call(&state, "send_companion_message", message("What's up?"));
        assert!(sent.await.unwrap()["runId"].is_string());

        let notify = call(&state, "set_companion_notify", json!({ "notify": false }));
        notify.await.unwrap();
        let snapshot = call(&state, "get_companion_snapshot", json!({ "dayStart": 0 }));
        assert_eq!(snapshot.await.unwrap()["notify"], false);
        state.shutdown();
    }
}
