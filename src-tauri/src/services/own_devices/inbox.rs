//! What a paired device reaches of the Inbox: the approvals waiting, the
//! open to-dos (read only), and approving or declining an approval
//! through the same service as its button, with the decisions record
//! naming the device.

use futures_util::future::BoxFuture;
use futures_util::FutureExt;

use super::wire::{self, PhoneTodo};
use crate::model::{ApprovalDecision, PendingApproval};
use crate::services::gateway::assignments::AssignmentStatus;
use crate::services::AppState;

/// The Inbox, as a device's commands use it
pub trait PhoneInbox: Send + Sync {
    /// Every pending tool approval, oldest first
    fn approvals(&self) -> BoxFuture<'_, Result<Vec<PendingApproval>, String>>;

    /// The open to-dos, oldest first
    fn todos(&self) -> BoxFuture<'_, Result<Vec<PhoneTodo>, String>>;

    /// Decides approval `id`; the record names `device`
    fn decide(
        &self,
        id: String,
        decision: ApprovalDecision,
        device: String,
    ) -> BoxFuture<'_, Result<(), String>>;
}

/// The app's own Inbox
pub struct AppInbox {
    state: AppState,
}

impl AppInbox {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

impl PhoneInbox for AppInbox {
    fn approvals(&self) -> BoxFuture<'_, Result<Vec<PendingApproval>, String>> {
        crate::services::chat::list_pending_approvals(&self.state).boxed()
    }

    fn todos(&self) -> BoxFuture<'_, Result<Vec<PhoneTodo>, String>> {
        async move {
            let todos = crate::services::gateway::todos::list(&self.state).await?;
            Ok(todos
                .into_iter()
                .filter(|todo| todo.status == AssignmentStatus::Open)
                .map(|todo| PhoneTodo {
                    title: wire::short(&todo.request, wire::MAX_TITLE_CHARS),
                    kind: serde_json::to_value(todo.kind)
                        .ok()
                        .and_then(|kind| kind.as_str().map(str::to_string))
                        .unwrap_or_default(),
                    from: todo.caller_name,
                    due_at: todo.due_at,
                    id: todo.task_id,
                })
                .collect())
        }
        .boxed()
    }

    fn decide(
        &self,
        id: String,
        decision: ApprovalDecision,
        device: String,
    ) -> BoxFuture<'_, Result<(), String>> {
        crate::services::inbox::audit::on_device(
            device,
            crate::services::chat::approve_action(&self.state, None, id, decision),
        )
        .boxed()
    }
}
