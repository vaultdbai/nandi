//! Tauri adapter: IPC commands for the desktop webview.
//!
//! Commands are thin wrappers over `services/` (1–3 lines each). The
//! exceptions are desktop-only features with no service equivalent: quick
//! pane, native notifications, the native file picker, and window state.

pub mod agents;
pub mod attachments;
pub mod aws_auth;
pub mod capabilities;
pub mod chat;
pub mod companion;
pub mod connectors;
pub mod docker;
pub mod email;
pub mod events;
pub mod gateway;
pub mod llm_keys;
pub mod marketplace;
pub mod notifications;
pub mod pilots;
pub mod recovery;
pub mod relay;
pub mod repo;
pub mod snapshots;
pub mod triggers;
pub mod workflows;
