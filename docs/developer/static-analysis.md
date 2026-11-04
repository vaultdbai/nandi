# Static Analysis

All static analysis tools configured in this app and how to use them.

## Quick Reference

| Tool           | Purpose                  | Command                   | In check:all |
| -------------- | ------------------------ | ------------------------- | ------------ |
| TypeScript     | Type checking            | `pnpm run typecheck`      | Yes          |
| ESLint         | Syntax, style, TS rules  | `pnpm run lint`           | Yes          |
| Prettier       | Code formatting          | `pnpm run format:check`   | Yes          |
| ast-grep       | Architecture patterns    | `pnpm run ast:lint`       | Yes          |
| React Compiler | Automatic memoization    | Build-time                | Yes          |
| cargo fmt      | Rust formatting          | `pnpm run rust:fmt:check` | Yes          |
| clippy         | Rust linting             | `pnpm run rust:clippy`    | Yes          |
| Vitest         | Frontend tests           | `pnpm run test:run`       | Yes          |
| cargo test     | Rust tests               | `pnpm run rust:test`      | Yes          |
| knip           | Unused code detection    | `pnpm run knip`           | No           |
| jscpd          | Duplicate code detection | `pnpm run jscpd`          | No           |

## Running All Checks

```bash
pnpm run check:all    # Must pass before commits
pnpm run fix:all      # Auto-fix what can be fixed
```

## Tool Details

### ESLint

Handles syntax, style, and TypeScript-specific rules.

```bash
pnpm run lint        # Check for issues
pnpm run lint:fix    # Auto-fix issues
```

Configuration in `eslint.config.js`.

### Prettier

Consistent code formatting.

```bash
pnpm run format:check   # Check formatting
pnpm run format         # Fix formatting
```

Configuration in `prettier.config.js`.

### ast-grep

Enforces architectural patterns ESLint can't detect. Catches violations like Zustand destructuring and hooks in wrong directories.

```bash
pnpm run ast:lint    # Scan for violations
pnpm run ast:fix     # Auto-fix where possible
```

**Key rules:**

- No Zustand destructuring (causes render cascades)
- Hooks must be in `hooks/` directory
- No store subscriptions in `lib/`
- No raw Tailwind palette colours (`text-emerald-500`) or hex values (`bg-[#ff544d]`) in `src/components/**` (`theme/no-raw-palette-colors`; `ui/` and the macOS traffic lights are exempt). Use the status and kind tokens instead
- Only `src-tauri/src/services/relay/client.rs` talks to the relay (`relay/relay-connection-only-in-client`): no other Rust file opens a WebSocket (`connect_async`, `connect_async_tls_with_config`, `client_async`) or builds the wire envelope (`RelayMessage::…`). The relay client itself is built only in `services/relay/mod.rs` (`relay/relay-client-built-only-in-relay`: `WsRelayClient::new` / `with_options`). Test modules (`mod tests`, `relay/tests.rs`) are exempt. Everything else goes through `state.relay_client()` and `relay::unavailable::{connected, configured}`, which answer `RelayUnavailable`

See [writing-ast-grep-rules.md](./writing-ast-grep-rules.md) for creating new rules.

### React Compiler

Handles memoization automatically at build time. You do **not** need to manually add:

- `useMemo` for computed values
- `useCallback` for function references
- `React.memo` for components

The compiler analyzes code and adds memoization where beneficial.

**Note:** The `getState()` pattern is still critical - it avoids store subscriptions, not memoization. See [state-management.md](./state-management.md).

### Rust Tooling

```bash
pnpm run rust:fmt:check   # Check formatting
pnpm run rust:fmt         # Fix formatting
pnpm run rust:clippy      # Lint with clippy
pnpm run rust:clippy:fix  # Auto-fix clippy warnings
pnpm run rust:test        # Run Rust tests
```

### knip (Periodic Cleanup)

Detects unused exports, dependencies, and files. Not in `check:all` - use periodically.

```bash
pnpm run knip
```

### jscpd (Periodic Cleanup)

Detects duplicated code blocks. Not in `check:all` - use periodically.

```bash
pnpm run jscpd
```

Use the `/cleanup` command for guided analysis and cleanup of both knip and jscpd findings.

## CI Integration

`.github/workflows/ci.yml` runs the `check:all` gates on every pull request and on pushes to `main` (ubuntu-latest): typecheck, lint, ast-grep, format check, `cargo fmt --check`, clippy with `-D warnings`, Vitest, `cargo test` and the offline suite (`pnpm run test:offline`, also part of `cargo test`). Each gate is its own step, so a failure names the gate. It installs Tauri's Linux system packages, takes the Rust toolchain from `rust-toolchain.toml`, and runs `pnpm run build` before the Rust steps because `tauri::generate_context!` needs `dist/` to exist.

Ensure it passes locally before pushing:

```bash
pnpm run check:all
```

## Adding New Rules

**ESLint:** Add rules to `eslint.config.js`

**ast-grep:** Create YAML files in `.ast-grep/rules/`. See [writing-ast-grep-rules.md](./writing-ast-grep-rules.md).

**Prettier:** Modify `prettier.config.js`
