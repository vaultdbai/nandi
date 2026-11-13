//! Tauri adapter: IPC commands for the desktop webview.
//!
//! Commands are thin wrappers over `services/` (1–3 lines each). The
//! exceptions are desktop-only features with no service equivalent: quick
//! pane, native notifications, the native file picker, and window state.

pub mod access;
pub mod agents;
pub mod ask;
pub mod attachments;
pub mod aws_auth;
pub mod bedrock;
pub mod capabilities;
pub mod chat;
pub mod companion;
pub mod connectors;
pub mod delegation;
pub mod delegation_clarification;
pub mod docker;
pub mod email;
pub mod events;
pub mod gateway;
pub mod groups;
pub mod harnesses;
pub mod identity;
pub mod images;
pub mod llm_keys;
pub mod machine;
pub mod manual_steps;
pub mod marketplace;
pub mod network;
pub mod notifications;
pub mod pilots;
pub mod preferences;
pub mod reassign;
pub mod recovery;
pub mod relay;
pub mod remote;
pub mod repo;
pub mod snapshots;
pub mod studio;
pub mod tasks;
pub mod teams;
pub mod teams_roster;
pub mod triggers;
pub mod workflows;
pub mod workspace;
