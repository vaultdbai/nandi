//! What a paired device reaches of the Inbox: the approvals waiting and
//! the open to-dos; approving or declining an approval, and marking a
//! to-do done or recording it handed, through the same services as their
//! buttons, with the decisions record naming the device.

use futures_util::future::BoxFuture;
use futures_util::FutureExt;

use super::wire::{self, PhoneTodo};
use crate::model::{ApprovalDecision, PendingApproval};
use crate::services::builtin_agents::DEFAULT_AGENT;
use crate::services::gateway::assignments::AssignmentStatus;
use crate::services::gateway::todos::{self, Actor};
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

    /// The whole sentence of open to-do `id` (`None`: not open)
    fn todo_text(&self, id: String) -> BoxFuture<'_, Result<Option<String>, String>>;

    /// Marks open to-do `id` done; the record names `device`
    fn todo_done(&self, id: String, device: String) -> BoxFuture<'_, Result<(), String>>;

    /// Records open to-do `id` handed to Nandi, whose run `run_id` works
    /// on it; the record names `device`
    fn todo_handed(
        &self,
        id: String,
        run_id: String,
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
                    working: todo.run_status.is_some_and(|s| !s.is_finished()),
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

    fn todo_text(&self, id: String) -> BoxFuture<'_, Result<Option<String>, String>> {
        async move {
            let todos = todos::list(&self.state).await?;
            Ok(todos
                .into_iter()
                .find(|todo| todo.task_id == id && todo.status == AssignmentStatus::Open)
                .map(|todo| todo.request))
        }
        .boxed()
    }

    fn todo_done(&self, id: String, device: String) -> BoxFuture<'_, Result<(), String>> {
        async move {
            crate::services::inbox::audit::on_device(
                device,
                todos::done(&self.state, id, Actor::User, None),
            )
            .await
            .map(|_| ())
        }
        .boxed()
    }

    fn todo_handed(
        &self,
        id: String,
        run_id: String,
        device: String,
    ) -> BoxFuture<'_, Result<(), String>> {
        async move {
            crate::services::inbox::audit::on_device(
                device,
                todos::handed(&self.state, id, DEFAULT_AGENT.to_string(), run_id),
            )
            .await
            .map(|_| ())
        }
        .boxed()
    }
}
