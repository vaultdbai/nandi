//! Calendar check commands (Tauri adapter for `services::workspace`). Each
//! account's files and calendar are connected with it (`rpc::email`).

use crate::services::workspace::{self, WorkspaceStatus};

super::commands! {
    fn get_workspace_status(state) -> WorkspaceStatus => Ok(workspace::status(&state).await);

    fn check_calendar_now(state) -> WorkspaceStatus => Ok(workspace::check_now(&state).await);

    fn set_calendar_interval(state, minutes: u32) -> WorkspaceStatus => workspace::set_calendar_interval(&state, minutes).await;
}
