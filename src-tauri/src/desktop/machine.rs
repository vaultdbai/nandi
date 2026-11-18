//! "Get your computer ready": the go-ahead that starts the installs.
//! Desktop-only: it starts installers (`winget`, `wsl`, Docker's) that
//! raise an OS admin prompt on this computer, so it runs only from the
//! desktop window, never over `POST /api/rpc`. The plan and its progress
//! are RPC (`rpc::machine`).

use tauri::State;

use crate::services::machine::{install, ReadyStep};
use crate::services::AppState;

/// The one go-ahead: runs the report's `plan` (`steps`, as the user saw
/// them) in the background; progress arrives as `machine://ready`. The
/// installers ask the OS for admin rights themselves
#[tauri::command]
#[specta::specta]
pub async fn get_computer_ready(
    state: State<'_, AppState>,
    steps: Vec<ReadyStep>,
) -> Result<(), String> {
    install::start(&state, steps).await
}
