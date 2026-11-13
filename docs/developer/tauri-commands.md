# Tauri Commands (tauri-specta)

Type-safe Tauri command bindings using [tauri-specta](https://github.com/specta-rs/tauri-specta).

## Overview

This app uses tauri-specta to generate TypeScript bindings from Rust commands, providing:

- **Compile-time type checking** - TypeScript catches errors before runtime
- **Auto-generated types** - No manual sync between Rust and TypeScript
- **IDE autocomplete** - Full IntelliSense for command names, parameters, and return types
- **Safe refactoring** - Rename commands safely across the stack

## Usage

### Calling Commands

```typescript
import { commands, type AppPreferences } from '@/lib/tauri-bindings'

// Commands return Result types for error handling
const result = await commands.loadPreferences()

if (result.status === 'ok') {
  console.log(result.data.theme) // Type-safe access
} else {
  console.error(result.error) // Type-safe error
}
```

### Result Type Pattern

Commands that can fail return a `Result<T, E>` type:

```typescript
type Result<T, E> = { status: 'ok'; data: T } | { status: 'error'; error: E }
```

See [error-handling.md](./error-handling.md) for comprehensive error handling patterns including structured error types, retry logic, and user feedback.

Handle both cases:

```typescript
const result = await commands.savePreferences({ theme: 'dark' })

if (result.status === 'error') {
  toast.error('Failed to save', { description: result.error })
  return
}

// result.data is available here
toast.success('Saved!')
```

### unwrapResult Helper

For cases where you want errors to propagate (throw) rather than handle them inline, use the `unwrapResult` helper:

```typescript
import { commands, unwrapResult } from '@/lib/tauri-bindings'

// Throws on error, returns data on success
const preferences = unwrapResult(await commands.loadPreferences())
```

**When to use each pattern:**

| Pattern          | Use When                                                        |
| ---------------- | --------------------------------------------------------------- |
| `unwrapResult`   | TanStack Query functions, errors should propagate to a boundary |
| Manual `if/else` | Event handlers, need explicit error handling (toasts, UI state) |

**TanStack Query example** (preferred pattern for data fetching):

```typescript
import { useQuery } from '@tanstack/react-query'
import { commands, unwrapResult } from '@/lib/tauri-bindings'

const { data, error } = useQuery({
  queryKey: ['preferences'],
  queryFn: async () => unwrapResult(await commands.loadPreferences()),
})
// TanStack Query handles the thrown error automatically
```

**Event handler example** (explicit error handling):

```typescript
const handleSave = async () => {
  const result = await commands.savePreferences(preferences)
  if (result.status === 'error') {
    toast.error('Failed to save', { description: result.error })
    return
  }
  toast.success('Preferences saved!')
}
```

## Adding New Commands

One rule: **declare each command once.** The declaration gives the Tauri command (desktop) and `POST /api/rpc/{name}` (browser), so the transports can't drift.

### 1. Declare it in one line

Put the logic in a service that takes `&AppState` and returns `Result<T, String>`. Then add one line to `src-tauri/src/rpc/<module>.rs` (the forms are documented in `rpc/mod.rs`):

```rust
super::commands! {
    /// One Request with its Steps
    fn get_request(state, id: String) -> Request => requests::get_request(&state, id).await;
}
```

A new module also gets `pub mod <module>;` and its `RPC` table in `TABLES` in `rpc/mod.rs`.

| ✅ Do                                              | ❌ Don't                                         |
| -------------------------------------------------- | ------------------------------------------------ |
| Validate and check access in the service           | Check arguments in a handler or the client       |
| One `rpc::commands!` line per command              | A Tauri command plus an HTTP route for one thing |
| A desktop-only command in `desktop/`, with its why | A hand-written command that could be RPC         |

### 2. Desktop-only commands

A command that needs the `AppHandle`, a window or a typed error is written by hand in `src-tauri/src/desktop/`. Add a row to the table in `desktop/mod.rs` saying why, and its name to `desktop::COMMANDS`:

```rust
#[tauri::command]
#[specta::specta]
pub async fn my_new_command(app: AppHandle, arg: String) -> Result<MyType, String> {
    // implementation
}
```

The browser can't call it, so add it to `HandWrittenCommands` in `src/lib/client/types.ts` and answer it in `src/lib/client/web.ts` (usually "use the desktop app").

### 3. Add Type derive to structs

```rust
use specta::Type;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct MyType {
    pub field: String,
}
```

### 4. Register in bindings.rs

Add it to `collect_commands!` in `src-tauri/src/bindings.rs` (`get_request` as `requests::get_request`). A test in `rpc/mod.rs` fails if an RPC command is missing here, or if a generated command is neither RPC nor in `desktop::COMMANDS`.

### 5. Regenerate TypeScript bindings

```bash
pnpm run rust:bindings
```

This runs `cargo test export_bindings -- --ignored` which generates `src/lib/bindings.ts`.

### 6. Use in frontend

```typescript
import { client } from '@/lib/client'

const result = await client.getRequest('r1')
```

The generated commands call `invoke` from `src/lib/client/invoke.ts`: Tauri IPC on the desktop, `POST /api/rpc/{name}` in a browser.

### 7. Commit both files

Always commit the Rust changes and the generated `src/lib/bindings.ts`.

## File Structure

```
src-tauri/src/
├── rpc/                # One line per command (rpc::commands!), both transports
├── desktop/            # Desktop-only Tauri commands, and why each is
├── http/               # The web server: /api/rpc, webhooks, emergency data, /ws
└── bindings.rs         # Command registration + export test

src/lib/
├── bindings.ts         # Generated (DO NOT EDIT)
├── tauri-bindings.ts   # Re-exports with project conventions
└── client/             # The transport, and the web client's hand-written answers
```

## Known Limitations

### serde_json::Value becomes JsonValue/unknown

Commands using `serde_json::Value` (like `saveEmergencyData`) have `JsonValue` typed parameters:

```typescript
type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | Partial<{ [key in string]: JsonValue }>
```

Cast when needed:

```typescript
await commands.saveEmergencyData(filename, data as JsonValue)
```

### Bindings generated at runtime

TypeScript bindings are generated when the app runs in debug mode, or via:

```bash
pnpm run rust:bindings
```

This must be run after changing Rust commands.

## Testing

Mock the commands in tests:

```typescript
// src/test/setup.ts
vi.mock('@/lib/tauri-bindings', () => ({
  commands: {
    loadPreferences: vi
      .fn()
      .mockResolvedValue({ status: 'ok', data: { theme: 'system' } }),
    savePreferences: vi.fn().mockResolvedValue({ status: 'ok', data: null }),
    // ... other commands
  },
}))
```

## Available Commands

Every command is in the generated `src/lib/bindings.ts`. The RPC ones are the `rpc/` modules; the desktop-only ones are listed in `src-tauri/src/desktop/mod.rs`.

## Dependencies

```toml
# src-tauri/Cargo.toml
specta = { version = "=2.0.0-rc.22", features = ["derive", "serde_json"] }
tauri-specta = { version = "=2.0.0-rc.21", features = ["typescript"] }
specta-typescript = "=0.0.9"
```

Note: Using exact versions (`=`) during RC phase to prevent breaking changes.

## References

- [tauri-specta GitHub](https://github.com/specta-rs/tauri-specta)
- [Specta documentation](https://specta.dev/docs/tauri-specta/v2)
