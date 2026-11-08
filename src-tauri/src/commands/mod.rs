//! Tauri adapter: IPC commands for the desktop webview.
//!
//! Commands are thin wrappers over `services/` (1–3 lines each). The
//! exceptions are desktop-only features with no service equivalent: quick
//! pane, native notifications, the native file picker, and window state.

pub mod access;
pub mod agents;
pub mod analysis;
pub mod ask;
pub mod attachments;
pub mod audit;
pub mod aws_auth;
pub mod bedrock;
pub mod capabilities;
pub mod capability;
pub mod capability_registries;
pub mod chat;
pub mod companion;
pub mod config;
pub mod connectors;
pub mod delegation;
pub mod delegation_clarification;
pub mod docker;
pub mod egress;
pub mod email;
pub mod events;
pub mod gateway;
pub mod groups;
pub mod harness_sessions;
pub mod harnesses;
pub mod identity;
pub mod images;
pub mod live;
pub mod llm_keys;
pub mod manual_steps;
pub mod marketplace;
pub mod marketplace_search;
pub mod mascot;
pub mod memory;
pub mod network;
pub mod notes;
pub mod notifications;
pub mod palette;
pub mod pilots;
pub mod policies;
pub mod preferences;
pub mod queue;
pub mod reassign;
pub mod recovery;
pub mod relay;
pub mod relay_capabilities;
pub mod remote;
pub mod repo;
pub mod requests;
pub mod runs;
pub mod shared;
pub mod snapshots;
pub mod studio;
pub mod tasks;
pub mod teams;
pub mod teams_roster;
pub mod tools;
pub mod triggers;
pub mod work;
pub mod workflow_builder;
pub mod workflows;
pub mod workspace;
