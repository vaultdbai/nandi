//! Task reassignment commands (Tauri adapter for `services::a2a::reassign`).

use crate::services::a2a::reassign;
use crate::services::a2a::reassign::{ReassignCandidate, ReassignRequest};
use crate::services::a2a::task::TaskRecord;

super::commands! {
    /// Hands a task we're serving to a teammate; returns our (now
    /// `reassigned`) row
    fn reassign_task(state, request: ReassignRequest) -> TaskRecord => reassign::reassign_task(&state, request).await;

    /// Online teammates who serve the task's capability
    fn list_reassign_candidates(state, task_id: String) -> Vec<ReassignCandidate> => reassign::reassign_candidates(&state, &task_id).await;
}
