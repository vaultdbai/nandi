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
//! `Result<Ok, String>`. Over HTTP the arguments are a JSON object with
//! camelCase keys, the same keys Tauri IPC uses.
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
        fn $name:ident($state:ident $(, $arg:ident: $ty:ty)* $(,)?) -> $ok:ty => $call:expr;
    )*) => {
        $(
            $(#[doc = $doc])*
            #[tauri::command]
            #[specta::specta]
            pub async fn $name(
                $state: tauri::State<'_, $crate::services::AppState>,
                $($arg: $ty),*
            ) -> Result<$ok, String> {
                $call
            }
        )*

        /// This module's commands over HTTP (`POST /api/rpc/{name}`)
        pub const RPC: &[(&str, $crate::rpc::Handler)] = &[$(
            (
                stringify!($name),
                |$state: $crate::services::AppState, args: serde_json::Value| -> $crate::rpc::RpcFuture {
                    Box::pin(async move {
                        #[derive(serde::Deserialize)]
                        #[serde(rename_all = "camelCase")]
                        struct Args { $($arg: $ty),* }

                        let Args { $($arg),* } = serde_json::from_value(args).map_err(|e| {
                            let name = stringify!($name);
                            format!("Invalid arguments for {name}: {e}")
                        })?;
                        let ok: $ok = $call?;
                        serde_json::to_value(ok).map_err(|e| e.to_string())
                    })
                },
            ),
        )*];
    };
}
pub(crate) use commands;

pub mod requests;

/// What an RPC call returns: the command's JSON result, or its error text
pub type RpcFuture = Pin<Box<dyn Future<Output = Result<Value, String>> + Send>>;

/// One command over HTTP: its JSON arguments in, its result out
pub type Handler = fn(AppState, Value) -> RpcFuture;

/// Every module's `RPC` table. A new module adds its table here.
const TABLES: &[&[(&str, Handler)]] = &[requests::RPC];

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
