# Rust Architecture

Module organization and patterns for the Tauri backend.

## Module Structure

```
src-tauri/src/
├── main.rs          # Entry point (just calls lib::run())
├── lib.rs           # App setup, plugins, startup logic
├── bindings.rs      # tauri-specta command registration
├── types.rs         # Shared types, constants, validation
├── services/        # The core: business logic, no Tauri types
├── rpc/             # Each command declared once (rpc::commands!)
├── desktop/         # Desktop-only Tauri commands, and the webview event sink
├── http/            # Web server: POST /api/rpc/{name}, webhooks, /ws
└── utils/           # Utility modules
    ├── mod.rs
    └── platform.rs  # Platform-specific helpers
```

## Adding New Commands

### 1. Declare it once

Put the logic in a service, then add one line to `src-tauri/src/rpc/<module>.rs`. It serves Tauri IPC and `POST /api/rpc/{name}` alike:

```rust
super::commands! {
    /// One Request with its Steps
    fn get_request(state, id: String) -> Request => requests::get_request(&state, id).await;
}
```

Only a command that needs the `AppHandle`, a window or a typed error is written by hand, in `desktop/` (see [Tauri Commands](./tauri-commands.md#2-desktop-only-commands)).

### 2. Wire a new module

A new `rpc/` module gets `pub mod <module>;` and its `RPC` table in `TABLES` in `rpc/mod.rs`.

### 3. Register in bindings.rs

```rust
pub fn generate_bindings() -> Builder<tauri::Wry> {
    use crate::rpc::{requests, /* ... */};

    Builder::<tauri::Wry>::new().commands(collect_commands![
        requests::get_request,
        // ... other commands
    ])
}
```

### 4. Regenerate TypeScript bindings

```bash
pnpm run rust:bindings
```

## Type Patterns

### Shared Types (types.rs)

Types shared between commands go in `types.rs`:

```rust
use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MyData {
    pub field: String,
}
```

**Note:** `#[derive(Type)]` from specta is required for TypeScript generation.

### Error Types

Most commands return `Result<T, String>`, which `rpc::commands!` serves on both transports. Use a typed enum only for errors the frontend must match on; such a command is written by hand in `desktop/` with its own browser route, as recovery is (`http/recovery.rs`):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "type")]
pub enum MyError {
    NotFound,
    ValidationError { message: String },
    IoError { message: String },
}
```

The `#[serde(tag = "type")]` makes errors easy to match in TypeScript:

```typescript
if (error.type === 'ValidationError') {
  console.log(error.message)
}
```

### Validation Functions

Keep validation in `types.rs` for reuse:

```rust
pub fn validate_input(input: &str) -> Result<(), String> {
    if input.is_empty() {
        return Err("Input cannot be empty".to_string());
    }
    Ok(())
}
```

### Wire Types (`nandi-protocol`)

Everything the relay and its clients exchange lives in the `nandi-protocol` crate of [vaultdbai/nandi-spec](https://github.com/vaultdbai/nandi-spec) (MIT), so the relay never depends on this app: `RelayMessage`, `PeerMessage`, the `/health`, `/instances` and `/relay/enroll` bodies, `split_token`, and the data they carry (`AgentCard`, `GatewayCard`, `Task`, `TaskResult`, `TaskState`, `ReasonKind`, `ProviderKind`, `Timestamp`, clarification questions).

- The app re-exports each type at its old path (`services::a2a::Task`, `model::Timestamp`, `services::relay::protocol::RelayMessage`), so imports don't change. Nothing else is re-exported or aliased.
- The crate holds data and pure methods only. App logic on these types is a free function in the app (`domain::agent_card::card_from_agent`).
- `specta::Type` comes from the crate's `specta` feature, which `src-tauri/Cargo.toml` enables.
- `src-tauri/Cargo.toml` depends on the git repo and `[patch]`es it to the sibling checkout `../nandi-spec`: clone nandi-spec next to this repo. A wire change is made there first (run its `cargo test` and `cargo clippy`), then used here. `src-tauri/.cargo/config.toml` sets `git-fetch-with-cli` so the private repo fetches with your git credentials.
- Serde shapes are the wire format: never rename or retype a field without a protocol version bump; add fields with `#[serde(default)]`.
- The client trait (`RelayClient`, `Handler`) is app code, in `services/relay/client_api.rs`.

### The Relay Server Is Another Repo

The relay server, its CLI, Docker image, Helm chart and the 20-user licence cap live in [vaultdbai/nandi-relay](https://github.com/vaultdbai/nandi-relay) (BUSL-1.1). This repo has only the client (`services/relay/client.rs`, `client_api.rs`, `presence.rs`, the client half of `tls.rs`).

- The runner never depends on the relay at build or run time: `nandi-relay` is a `[dev-dependencies]` entry only, so the client's integration tests (`services/relay/tests.rs`, `presence.rs`) can start a real relay (`nandi_relay::{serve, serve_open, serve_with, issue_instance_token}`). `cargo tree -e normal -i nandi-relay` must print nothing.
- Like nandi-spec, it's a git dependency `[patch]`ed to a sibling checkout, `../nandi-relay`: clone it next to this repo to run the Rust tests.
- Never add licence checks here; a relay's refusal (for example a full relay) reaches the user as the relay's own message.

## Platform-Specific Code

Use conditional compilation for platform-specific behavior:

```rust
#[cfg(target_os = "macos")]
fn macos_specific() { /* ... */ }

#[cfg(desktop)]
fn desktop_only() { /* ... */ }

#[cfg(not(target_os = "linux"))]
fn non_linux() { /* ... */ }
```

## Plugin Registration (lib.rs)

Plugins are registered in `lib.rs` during app setup:

```rust
// Desktop-only plugins
#[cfg(desktop)]
{
    app_builder = app_builder.plugin(tauri_plugin_window_state::Builder::new().build());
}

// All platforms
app_builder = app_builder
    .plugin(tauri_plugin_fs::init())
    .plugin(tauri_plugin_dialog::init())
```

**Order matters:** Single-instance plugin must be registered first.

## Conventions

| Pattern           | Example                                                       |
| ----------------- | ------------------------------------------------------------- |
| Command naming    | `snake_case` (`load_preferences`, not `loadPreferences`)      |
| Error returns     | `Result<T, String>` for simple errors, typed enum for complex |
| Logging           | Use `log::info!`, `log::debug!`, etc.                         |
| String formatting | `format!("{variable}")` not `format!("{}", variable)`         |
| App handle        | Pass `AppHandle` not `Window` when possible                   |

## Expanding This Architecture

When adding new features:

1. **New command domain?** Create a new file in `rpc/` (or `desktop/` if it needs the `AppHandle` or a window)
2. **New shared types?** Add to `types.rs`
3. **Platform-specific code?** Use `#[cfg(target_os = "...")]` next to the code that needs it
4. **New plugin?** Register in `lib.rs` setup
