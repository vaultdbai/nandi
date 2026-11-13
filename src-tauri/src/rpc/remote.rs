//! Git remote commands (Tauri adapter for `services::remote`). The
//! credential goes in once and never comes back out.

use crate::services::remote;
use crate::services::remote::{PullResult, PushResult, RemoteCredential, RemoteStatus};

super::commands! {
    fn get_remote_status(state) -> RemoteStatus => remote::get_remote_status(&state).await;

    /// Pushes go to `<prefix>/<slug>` only (default prefix `nandi`)
    fn configure_remote(state, url: String, credential: Option<RemoteCredential>, prefix: Option<String>, slug: Option<String>) -> RemoteStatus => remote::configure_remote(&state, url, credential, prefix, slug).await;

    fn clear_remote(state) -> () => remote::clear_remote(&state).await;

    fn test_remote_connection(state) -> () => remote::test_remote_connection(&state).await;

    /// Never to main or master. `overwrite` force-pushes `<prefix>/<slug>`.
    fn push_to_remote(state, overwrite: bool) -> PushResult => remote::push_to_remote(&state, overwrite).await;

    fn pull_from_remote(state) -> PullResult => remote::pull_from_remote(&state).await;

    fn list_remote_branches(state) -> Vec<String> => remote::list_remote_branches(&state).await;
}
