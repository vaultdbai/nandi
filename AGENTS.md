# AI Agent Instructions

## Overview

Nandi is the desktop agent runner (Apache-2.0): Tauri v2, React 19, Rust, pnpm. It works fully without a relay. The relay server is [vaultdbai/nandi-relay](https://github.com/vaultdbai/nandi-relay) (BUSL-1.1); the wire types are `nandi-protocol` in [vaultdbai/nandi-spec](https://github.com/vaultdbai/nandi-spec) (MIT). Inside the Nandi workspace, the workspace `AGENTS.md` adds the general standards.

## Core Rules

### New Sessions

- Read @docs/tasks.md for task management
- Review `docs/developer/architecture-guide.md` for high-level patterns
- Check `docs/developer/README.md` for the full documentation index
- Check git status and project structure
- Clone nandi-spec and nandi-relay next to this repo (`../`) to build and run the Rust tests. `src-tauri/Cargo.toml` patches them to `../nandi-spec` and `../nandi-relay`

### Development Practices

**CRITICAL:** Follow these strictly:

0. **Use pnpm only**: This project uses `pnpm`, NOT `npm`. Always use `pnpm install`, `pnpm run`, etc. Node is pinned in `.node-version` (24), and CI and release builds read it: Node 26 ships its own `localStorage`, which breaks the jsdom tests.
1. **Read Before Editing**: Always read files first to understand context
2. **Follow Established Patterns**: Use patterns from this file and `docs/developer`
3. **Match Code Style**: Follow existing formatting and patterns
4. **Test Coverage**: Write comprehensive tests for business logic
5. **Quality Gates**: Run `pnpm run check:all` after significant changes, and `pnpm run test:offline` when relay or startup code changed
6. **No Dev Server**: Ask the user to run it and report back
7. **No Unsolicited Commits**: Only when explicitly requested. Never force-push
8. **Conventional Commits**: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`, with a scope where it helps (`fix(network): ...`)
9. **No Legacy Code**: Nothing is released, so no migrations, fallbacks or aliases. When renaming, just change it
10. **Clean OOP/OOD, MVC in the UI**: Clear objects and responsibilities, as pure as possible
11. **Documentation**: Update relevant `docs/developer/` files for new patterns
12. **Feature Test Catalog**: When user-visible behavior is added, changed or removed, update its entry in `docs/testing/` in the same change (see `docs/testing/README.md`)
13. **Removing files**: Always use `rm -f`

**CRITICAL:** Use Tauri v2 docs only. Always use modern Rust formatting: `format!("{variable}")`

### Product Rules (CRITICAL)

- **Agents run only in the container.** Nothing an agent does runs on the host or in a host coding CLI. Only connectors and pilots reach outside, called from inside the container or exposed to it. Don't default runs to `placement: host`. One exception: the visible `browser` pilot runs Chrome or Edge on the host so the user can watch, through the same egress proxy, allowed-sites check and per-call approvals as `browser-headless`. It is not a precedent.
- **Credentials never go into containers or images**: no `~/.claude/.credentials.json`, `~/.aws/credentials`, SSO caches, CLI auth files, or tokens in `pip.conf` or `.npmrc`. Pass a short-lived token or host-resolved STS keys per session. Non-secret settings are mounted read-only by default.
- **Company sign-in first** (Claude Team or Enterprise login, AWS SSO through Bedrock), API keys last. The free trial never goes ahead of the user's own keys or sign-ins.
- **Packs set no `model:`.** Pack agents use the user's default model.
- **Web search stays keyless**, limited to the agent's `network.allow` sites. Don't add key-based search providers without asking.
- **Chief of Staff** ships with the app as an editable local agent. Its behaviour lives in its `skill.md` (900 words or fewer), not in code.
- **Never ship `NANDI_RELAY_SECRET` in a client.** Builds carry `NANDI_DEFAULT_RELAY_URL` and a join code (`NANDI_RELAY_JOIN_CODE`).

## Architecture Patterns (CRITICAL)

### State Management Onion

```
useState (component) → Zustand (global UI) → TanStack Query (persistent data)
```

**Decision**: Is data needed across components? → Does it persist between sessions?

### Performance Pattern (CRITICAL)

```typescript
// ✅ GOOD: Selector syntax - only re-renders when specific value changes
const leftSidebarVisible = useUIStore(state => state.leftSidebarVisible)

// ❌ BAD: Destructuring causes render cascades (caught by ast-grep)
const { leftSidebarVisible } = useUIStore()

// ✅ GOOD: Use getState() in callbacks for current state
const handleAction = () => {
  const { data, setData } = useStore.getState()
  setData(newData)
}
```

### Gold Copy (CRITICAL)

Definitions (agents, workflows, triggers, portable settings, connectors) are **files** in `~/.nandi/repo/.nandi/`. SQLite is a rebuildable projection of them.

```rust
// ❌ BAD: writing a definition into SQLite — the next rebuild erases it
conn.execute("UPDATE agents SET name = ?1 WHERE id = ?2", params![name, id])?;

// ✅ GOOD: domain validates → repository writes the file → commit projects + publishes
store.repos().agents.save(&agent)?;
store.commit(EntityKind::Agent, agent.id().as_str())?;
```

- `services/domain/` is pure (no I/O); `services/repos/` are the only writers of repo files
- Secrets go in the OS keyring; files hold `secret_refs` only
- See `docs/developer/data-model.md` and `docs/developer/domain-model.md`

### Works Without the Relay (CRITICAL)

Nandi must be fully usable with no relay configured or reachable. This repo is the Apache-2.0 agent runner; the relay (`vaultdbai/nandi-relay`, BUSL-1.1) is an optional team layer.

- **Only person-to-person features need the relay**: asking or assigning someone else, the team inbox (requests from other people), reassignment and handover, finding owners and presence. Everything else works alone: agents (including the Chief of Staff), chat, runs, workflows, triggers, tools, pilots, models, Studio, backups, the personal inbox (your own agents' approvals) and **installing starter packs** (from the GitHub registry or a local folder, never through the relay).
- Talk to the relay only through `services/relay/client.rs`. No other module opens a connection to it or speaks its wire protocol; the client is built once, in `services/relay/mod.rs`. Enforced by the ast-grep rules in `.ast-grep/rules/relay/` (`pnpm run ast:lint`).
- Never block startup, a screen, a command or a run on the relay. Connect in the background; a build's default relay (`relay::BuiltInRelay`, `NANDI_DEFAULT_RELAY_URL` + `NANDI_RELAY_JOIN_CODE`) enrolls and connects in the background with backoff too.
- A connected feature without a relay returns the typed `RelayUnavailable` (`services/relay/unavailable.rs`; get the client with `unavailable::connected` / `configured`), never an ad-hoc string. It crosses the bridge as an error starting with `RELAY_UNAVAILABLE` (exported to `bindings.ts`; HTTP answers 503). The frontend recognises it with `isRelayUnavailable` (`lib/friendly-error.ts`) and shows `RelayConnectNotice` (`components/network/RelayConnectNotice.tsx`, or `toastPersonError` where there's no room) there and nowhere else, with no nags. Local-only actions never return it.
- Never put the 20-user cap or any licence check in this repo. It lives only in the relay.
- Mark every feature entry in `docs/testing/` with "Needs relay: yes / no". A "no" feature that breaks offline blocks a release.
- `pnpm run test:offline` (`src-tauri/src/offline_suite.rs`, in CI) boots the core with no relay and with an unreachable default relay: every screen answers, an agent runs and is approved, a workflow runs, a starter pack installs from a local folder, and person-to-person endpoints answer `RelayUnavailable`. Keep it green.

### Static Analysis

- **React Compiler**: Handles memoization automatically - no manual `useMemo`/`useCallback` needed. The ESLint `react-compiler` rule is an error
- **ESLint**: `--max-warnings 0`. A warning fails `check:all`
- **ast-grep** (`pnpm run ast:lint`, rules in `.ast-grep/rules/`, see `docs/developer/static-analysis.md`):

  | Rule file                                | What it enforces                                                 |
  | ---------------------------------------- | ---------------------------------------------------------------- |
  | `zustand/no-destructure`                 | Selector syntax, never destructure a store                       |
  | `architecture/hooks-in-hooks-dir`        | React hooks live in `src/hooks/` or `src/services/`, not `lib/`  |
  | `architecture/no-store-in-lib`           | `src/lib/` reads stores with `getState()` only, no subscriptions |
  | `theme/no-raw-palette-colors`            | Theme tokens only, no raw Tailwind palette colours or hex values |
  | `relay/relay-connection-only-in-client`  | Only `services/relay/client.rs` speaks to the relay              |
  | `relay/relay-client-built-only-in-relay` | The relay client is built only in `services/relay/mod.rs`        |

- **Prettier**: no semicolons, single quotes, width 80, LF, `proseWrap: preserve`. It checks Markdown too (`pnpm run format:check`)
- **Line endings**: LF everywhere (`.gitattributes`); `.bat`, `.cmd` and `.ps1` keep CRLF
- **CI** runs `pnpm run build` before clippy, so the frontend must build
- **Knip/jscpd**: Periodic cleanup tools (`/cleanup` in the Nandi workspace)

### Event-Driven Bridge

- **Rust → React**: `app.emit("event-name", data)` → `listen("event-name", handler)`
- **React → Rust**: Use typed commands from `@/lib/tauri-bindings` (tauri-specta)
- **Commands**: All actions flow through centralized command system

### Tauri Command Pattern (tauri-specta)

```typescript
// ✅ GOOD: Type-safe commands with Result handling
import { commands } from '@/lib/tauri-bindings'

const result = await commands.loadPreferences()
if (result.status === 'ok') {
  console.log(result.data.theme)
}

// ❌ BAD: String-based invoke (no type safety)
const prefs = await invoke('load_preferences')
```

**Adding commands**: See `docs/developer/tauri-commands.md`

### Internationalization (i18n)

```typescript
// ✅ GOOD: Use useTranslation hook in React components
import { useTranslation } from 'react-i18next'

function MyComponent() {
  const { t } = useTranslation()
  return <h1>{t('myFeature.title')}</h1>
}

// ✅ GOOD: Non-React contexts - bind for many calls, or use directly
import i18n from '@/i18n/config'
const t = i18n.t.bind(i18n)  // Bind once for many translations
i18n.t('key')                 // Or call directly for occasional use
```

- **Translations**: All strings in `/locales/*.json`
- **RTL Support**: Use CSS logical properties (`text-start` not `text-left`)
- **Adding strings**: See `docs/developer/i18n-patterns.md`

### Documentation & Versions

- **Context7 First**: Always use Context7 for framework docs before WebSearch
- **Version Requirements**: Tauri v2.x, shadcn/ui v4.x, Tailwind v4.x, React 19.x, Zustand v5.x, Vite v7.x, Vitest v4.x

## Developer Documentation

For complete patterns and detailed guidance, see `docs/developer/README.md`.

Key documents:

- `architecture-guide.md` - Mental models, security, anti-patterns
- `state-management.md` - State onion, getState() pattern details
- `tauri-commands.md` - Adding new Rust commands
- `static-analysis.md` - All linting tools and quality gates

## Claude Code Skills & Agents

Inside the Nandi workspace, the shared `/check` and `/cleanup` skills and the `cleanup-analyzer` and `userguide-reviewer` agents come from the workspace's `.claude/`. Outside it, run `pnpm run check:all` (and `pnpm run test:offline` for relay or startup changes) before you commit.

## Licence

Apache-2.0 (see `LICENSE` and `NOTICE`). The brand assets (the Nandi name, the bull, the logos and icons) are not covered: see `TRADEMARKS.md`. Never copy code from `nandi-relay` (BUSL-1.1) into this repo, and never put the 20-user cap or a licence check here.
