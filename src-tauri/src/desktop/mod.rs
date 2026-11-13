//! The desktop app's own Tauri side: the commands `rpc::commands!` can't
//! declare, and the event sink to the webview.
//!
//! Every other command is one line in `rpc/` and works on both transports.
//! A command belongs here only when it needs what a browser doesn't have:
//!
//! | Module          | Why it isn't RPC                                                   |
//! | --------------- | ------------------------------------------------------------------ |
//! | `attachments`   | `pick_attachments` opens the native file picker on this computer   |
//! | `companion`     | Shows, places and pins the companion window; the global hotkey     |
//! | `events`        | Not a command: emits core events to the webview (`TauriSink`)      |
//! | `notifications` | `send_native_notification` shows an OS notification                |
//! | `recovery`      | Typed errors (`RecoveryError`); the browser has `http/recovery.rs` |
//! | `run_files`     | `open_run_file` opens a run's file with its app on this computer   |
//!
//! The web client answers these itself (`DesktopCommands` in
//! `src/lib/client/types.ts`), mostly with "use the desktop app".

pub mod attachments;
pub mod companion;
pub mod events;
pub mod notifications;
pub mod recovery;
pub mod run_files;
