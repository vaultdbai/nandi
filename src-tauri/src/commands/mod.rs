//! Tauri adapter: IPC commands for the desktop webview.
//!
//! Commands are thin wrappers over `services/` (1–3 lines each). The
//! exceptions are desktop-only features with no service equivalent: quick
//! pane, native notifications, the native file picker, and window state.

pub mod agents;
pub mod attachments;
pub mod chat;
pub mod companion;
pub mod events;
pub mod notifications;
pub mod recovery;
pub mod snapshots;
pub mod workflows;
