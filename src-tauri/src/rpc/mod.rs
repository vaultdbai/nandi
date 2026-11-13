//! One declaration per command, served on both transports.
//!
//! [`commands!`] turns each line into:
//!
//! - the Tauri command (`#[tauri::command] #[specta::specta]`), which
//!   `bindings.rs` registers and exports to `src/lib/bindings.ts`, and
//! - a row of the module's `RPC` table, which `POST /api/rpc/{name}`
//!   (`http/rpc.rs`) serves to the web client.
//!
//! Both run the same service expression, so the transports can't drift.
//!
//! ```ignore
//! commands! {
//!     /// One Request with its Steps
//!     fn get_request(state, id: String) -> Request => requests::get_request(&state, id).await;
//! }
//! ```
//!
//! A line reads: the command's name, `state` (the `AppState`) and its
//! arguments, the `Ok` type, then the service call. The call must return
//! `Result<Ok, String>`; a sync service is called without `.await`. Over
//! HTTP the arguments are a JSON object with camelCase keys, the same keys
//! Tauri IPC uses.
//!
//! A command that doesn't read the app state leaves `state` out:
//!
//! ```ignore
//! commands! {
//!     fn help_topics() -> Vec<HelpTopic> => Ok(help::topics());
//! }
//! ```
//!
//! Commands that need the `AppHandle`, a window or a typed error stay
//! hand-written in `commands/`.

use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

use crate::services::AppState;

/// Declares commands; see the module docs.
macro_rules! commands {
    ($(
        $(#[doc = $doc:literal])*
        fn $name:ident($($params:tt)*) -> $ok:ty => $call:expr;
    )*) => {
        $(
            $crate::rpc::commands!(@command [$($doc)*] $name($($params)*) -> $ok => $call);
        )*

        /// This module's commands over HTTP (`POST /api/rpc/{name}`)
        pub const RPC: &[(&str, $crate::rpc::Handler)] = &[$(
            (stringify!($name), $crate::rpc::commands!(@handler $name($($params)*) -> $ok => $call)),
        )*];
    };

    // The Tauri command, with the app state
    (@command [$($doc:literal)*] $name:ident($state:ident $(, $arg:ident: $ty:ty)* $(,)?) -> $ok:ty => $call:expr) => {
        $(#[doc = $doc])*
        #[tauri::command]
        #[specta::specta]
        pub async fn $name(
            $state: tauri::State<'_, $crate::services::AppState>,
            $($arg: $ty),*
        ) -> Result<$ok, String> {
            $call
        }
    };

    // The Tauri command, without the app state
    (@command [$($doc:literal)*] $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ok:ty => $call:expr) => {
        $(#[doc = $doc])*
        #[tauri::command]
        #[specta::specta]
        pub async fn $name($($arg: $ty),*) -> Result<$ok, String> {
            $call
        }
    };

    // The RPC handler, with the app state
    (@handler $name:ident($state:ident $(, $arg:ident: $ty:ty)* $(,)?) -> $ok:ty => $call:expr) => {
        |$state: $crate::services::AppState, args: serde_json::Value| -> $crate::rpc::RpcFuture {
            Box::pin(async move {
                $crate::rpc::commands!(@args $name, args, $($arg: $ty),*);
                let result: Result<$ok, String> = $call;
                serde_json::to_value(result?).map_err(|e| e.to_string())
            })
        }
    };

    // The RPC handler, without the app state
    (@handler $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ok:ty => $call:expr) => {
        |_: $crate::services::AppState, args: serde_json::Value| -> $crate::rpc::RpcFuture {
            Box::pin(async move {
                $crate::rpc::commands!(@args $name, args, $($arg: $ty),*);
                let result: Result<$ok, String> = $call;
                serde_json::to_value(result?).map_err(|e| e.to_string())
            })
        }
    };

    // Binds each argument from the JSON object (camelCase keys)
    (@args $name:ident, $args:ident, $($arg:ident: $ty:ty),*) => {
        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Args { $($arg: $ty),* }

        let Args { $($arg),* } = serde_json::from_value($args).map_err(|e| {
            let name = stringify!($name);
            format!("Invalid arguments for {name}: {e}")
        })?;
    };
}
pub(crate) use commands;

pub mod access;
pub mod analysis;
pub mod ask;
pub mod audit;
pub mod aws_auth;
pub mod bedrock;
pub mod capability;
pub mod capability_definitions;
pub mod capability_registries;
pub mod config;
pub mod connectors;
pub mod containers;
pub mod decisions;
pub mod delegation;
pub mod delegation_clarification;
pub mod egress;
pub mod email;
pub mod gateway;
pub mod groups;
pub mod harness_sessions;
pub mod harnesses;
pub mod help;
pub mod identity;
pub mod images;
pub mod knowme;
pub mod live;
pub mod llm_keys;
pub mod machine;
pub mod manual_steps;
pub mod marketplace;
pub mod marketplace_search;
pub mod mascot;
pub mod memory;
pub mod network;
pub mod notes;
pub mod palette;
pub mod pilots;
pub mod policies;
pub mod preferences;
pub mod queue;
pub mod reassign;
pub mod relay;
pub mod relay_capabilities;
pub mod remote;
pub mod repo;
pub mod requests;
pub mod runs;
pub mod shared;
pub mod studio;
pub mod tasks;
pub mod teams;
pub mod teams_roster;
pub mod tools;
pub mod triggers;
pub mod work;
pub mod workflow_builder;
pub mod workspace;

/// What an RPC call returns: the command's JSON result, or its error text
pub type RpcFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;

/// One command over HTTP: its JSON arguments in, its result out
pub type Handler = fn(AppState, Value) -> RpcFuture;

/// Every module's `RPC` table. A new module adds its table here.
const TABLES: &[&[(&str, Handler)]] = &[
    access::RPC,
    analysis::RPC,
    ask::RPC,
    audit::RPC,
    aws_auth::RPC,
    bedrock::RPC,
    capability::RPC,
    capability_definitions::RPC,
    capability_registries::RPC,
    config::RPC,
    connectors::RPC,
    containers::RPC,
    decisions::RPC,
    delegation::RPC,
    delegation_clarification::RPC,
    egress::RPC,
    email::RPC,
    gateway::RPC,
    groups::RPC,
    harness_sessions::RPC,
    harnesses::RPC,
    help::RPC,
    identity::RPC,
    images::RPC,
    knowme::RPC,
    live::RPC,
    llm_keys::RPC,
    machine::RPC,
    manual_steps::RPC,
    marketplace::RPC,
    marketplace_search::RPC,
    mascot::RPC,
    memory::RPC,
    network::RPC,
    notes::RPC,
    palette::RPC,
    pilots::RPC,
    policies::RPC,
    preferences::RPC,
    queue::RPC,
    reassign::RPC,
    relay::RPC,
    relay_capabilities::RPC,
    remote::RPC,
    repo::RPC,
    requests::RPC,
    runs::RPC,
    shared::RPC,
    studio::RPC,
    tasks::RPC,
    teams::RPC,
    teams_roster::RPC,
    tools::RPC,
    triggers::RPC,
    work::RPC,
    workflow_builder::RPC,
    workspace::RPC,
];

/// Every command name with its handler
fn all() -> impl Iterator<Item = &'static (&'static str, Handler)> {
    TABLES.iter().copied().flatten()
}

/// Runs the command `name`, or `None` when there is no such command
pub async fn call(state: AppState, name: &str, args: Value) -> Option<Result<Value, String>> {
    let (_, handler) = all().find(|(command, _)| *command == name)?;
    Some(handler(state, args).await)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every RPC command is a specta command too, so desktop and web both
    /// reach it
    #[test]
    fn every_command_is_on_both_transports() {
        let bindings = crate::bindings::generate_bindings()
            .export_str(specta_typescript::Typescript::default())
            .unwrap();
        let names: Vec<&str> = all().map(|(name, _)| *name).collect();
        assert!(!names.is_empty());
        for name in &names {
            assert!(
                bindings.contains(&format!("TAURI_INVOKE(\"{name}\"")),
                "{name} is in an RPC table but not in bindings.rs"
            );
        }
        let mut unique = names.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "an RPC name is declared twice");
    }
}
