//! Calendar and files commands (Tauri adapter for `services::workspace`).

use crate::services::auth::oauth::DeviceFlowState;
use crate::services::email::EmailSignInPoll;
use crate::services::workspace::{self, AccountKind, WorkspaceStatus};

super::commands! {
    fn get_workspace_status(state) -> WorkspaceStatus => Ok(workspace::status(&state).await);

    /// Starts connecting a calendar or drive: only its own scopes are asked for
    fn start_workspace_sign_in(state, kind: AccountKind) -> DeviceFlowState => workspace::start_sign_in(&state, kind).await;

    fn poll_workspace_sign_in(state, kind: AccountKind) -> EmailSignInPoll => Ok(workspace::poll_sign_in(&state, kind).await);

    fn disconnect_workspace(state, kind: AccountKind) -> WorkspaceStatus => workspace::disconnect(&state, kind).await;

    fn check_calendar_now(state) -> WorkspaceStatus => Ok(workspace::check_now(&state).await);

    fn set_calendar_interval(state, minutes: u32) -> WorkspaceStatus => workspace::set_calendar_interval(&state, minutes).await;
}
