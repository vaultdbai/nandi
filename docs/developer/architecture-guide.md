# Architecture Guide

High-level architectural overview and mental models for this app.

## Philosophy

1. **Clarity over Cleverness** - Predictable patterns over magic
2. **AI-Friendly Architecture** - Clear patterns that AI agents can follow
3. **Performance by Design** - Patterns that prevent common performance pitfalls
4. **Security First** - Built-in security patterns for file system operations
5. **Extensible Foundation** - Easy to add new features without refactoring

## Mental Models

### The "Onion" State Architecture

State management follows a three-layer hierarchy:

```
┌─────────────────────────────────────┐
│           useState                  │  ← Component UI State
│  ┌─────────────────────────────────┐│
│  │          Zustand                ││  ← Global UI State
│  │  ┌─────────────────────────────┐││
│  │  │      TanStack Query         │││  ← Persistent Data
│  │  └─────────────────────────────┘││
│  └─────────────────────────────────┘│
└─────────────────────────────────────┘
```

**Decision Tree:**

```
Is this data needed across multiple components?
├─ No → useState
└─ Yes → Does this data persist between app sessions?
    ├─ No → Zustand
    └─ Yes → TanStack Query
```

See [state-management.md](./state-management.md) for implementation details.

### Event-Driven Bridge

Rust and React communicate through events for loose coupling:

```
Rust Menu Click → Event Emission → React Listener → Command Execution → State Update
Keyboard Shortcut → Event Handler → Command Execution → State Update
Command Palette → Command Selection → Command Execution → State Update
```

This ensures the same actions work consistently across all interaction methods.

### Command-Centric Design

All user actions flow through a centralized [command system](./command-system.md):

- **Commands** are pure objects with `execute()` functions
- **Context** provides all state and actions commands need
- **Registration** merges commands from different domains at runtime

This decouples UI triggers from implementations and enables consistent behavior.

## Pattern Dependencies

Understanding how patterns work together:

```
Command System
├── Depends on: State Management (context)
├── Integrates with: Keyboard Shortcuts, Menus
└── Enables: Consistent behavior across UI

State Management
├── Enables: Performance (getState pattern)
├── Supports: Data Persistence, UI State
└── Foundation for: All other systems

Event-Driven Bridge
├── Enables: Rust-React communication
├── Supports: Security (validation in Rust)
└── Foundation for: Menus, Updates, Notifications
```

## Core Systems

| System               | Documentation                                    |
| -------------------- | ------------------------------------------------ |
| Command System       | [command-system.md](./command-system.md)         |
| Keyboard Shortcuts   | [keyboard-shortcuts.md](./keyboard-shortcuts.md) |
| Native Menus         | [menus.md](./menus.md)                           |
| Companion Window     | [companion-window.md](./companion-window.md)     |
| Data Persistence     | [data-persistence.md](./data-persistence.md)     |
| Internationalization | [i18n-patterns.md](./i18n-patterns.md)           |
| Cross-Platform       | [cross-platform.md](./cross-platform.md)         |

## Component Hierarchy

```
MainWindow (App chrome)
├── TitleBar (Window controls + toolbar)
├── AppShell (Resizable three-pane layout)
│   ├── Sidebar (Seven pages as a tree: Home, Ask, Agents, Automations, Library, Network, Settings)
│   ├── MainPane (Current view from navigation-store; pages host tabs)
│   └── Context panel (workflow view only)
└── Global Overlays
    ├── CommandPalette (Cmd+K)
    └── Toaster (Notifications)
```

### Navigation

There is no router. `useNavigationStore` (`src/store/navigation-store.ts`) holds the current `View` — a discriminated union (`home`, `agent`, `workflow`, `studio`, `request`, …) — plus a bounded back stack. Navigate from anywhere with `useNavigationStore.getState().navigate(view)`; `MainPane` renders the matching component from `src/components/views/`. To deep-link into a conversation, pass `focusMessageId` on an `agent` view.

**State a page should keep across Back goes in its view.** Filters, for example: All activity's live in `{ kind: 'home-activity', filters }`. Change them with `useNavigationStore.getState().replace(view)`, which swaps the current view without adding a Back step, so Back from a row returns to the filtered list rather than walking through every filter change.

**Seven pages, two levels.** The sidebar's top level is exactly `NAV_SECTIONS` (Home, Ask, Agents, Automations, Library, Network, Settings), each opening `SECTION_VIEWS[item]`. `buildNavTree` (`src/components/layout/nav-tree.ts`, pure and tested) adds one level under three of them: Ask (New chat, which opens `{ kind: 'studio', conversationId: null, draft: true }`, an empty chat created when its first message is sent; the five newest chats; All chats), Agents (each agent, busiest first, cut off with All agents) and Automations (Workflows, Triggers). Home, Library, Network and Settings stay single items: their pages' tabs are enough (owner's call, 2026-10-21). The current node is the deepest one whose view holds the current one (`activePath`, `viewContains`: same kind, every field the node sets equal), so an agent's Runs tab lights that agent; All … rows never count (they open their page's own view), and views with no node fall back to `navSection`. Open branches live in `nav-tree-store` (saved to `localStorage`), and the branch above the current node opens by itself. a page with tabs (`NavPages.tsx`) keeps its tab in the view (`{ kind: 'library', tab: 'browse', filter: 'tools' }`), so tabs survive Back. A page's drawer is part of the view too (Library's `item`): open it with `navigate` so Back closes it, and close it with `replace` so closing doesn't add a history entry. `navSection(view)` says which item a view belongs to (an agent's page is under Agents, a Request under Home) and drives the sidebar's active state.

**Only current view kinds exist.** The `View` union lists the pages and their tabs (`home`, `home-activity`, `automations`, `library`, `network`, `settings`, …). `parseView` checks untrusted view JSON (palette results, the companion, deep links) against `VIEW_KINDS` and rejects any other kind, and the Rust palette (`services/palette.rs`) emits only current kinds. When you add a view, add its kind to `VIEW_KINDS` and give it a `navSection` case.

**Settings is a page, not a dialog.** To send the user to a setting, call `openSettings(section, { focus?, logTab?, agentId? })` (`src/lib/settings-nav.ts`): it navigates to `{ kind: 'settings', section }`, so "Add a model" lands on **Models** and Back works across sections. The sections are `SETTINGS_SECTIONS` (Models & runtimes, Account, Cloud credentials, Internet access, Backup, Sources, Logs, Appearance), each in `src/components/settings/sections/` inside `SettingsSectionFrame`, with technical material in an `AdvancedBlock`. `focus` scrolls to and highlights the element marked `data-settings-focus={focus}` (a provider, a runtime id, a source URL). Runtimes live on Models & runtimes: link to them with `openSettings('models', { focus: 'runtimes' })`. A plain `openSettings()` (sidebar, Ctrl+, the app menu) opens the last section used this session (`ui-store.settingsSection`). Each setting has one home: the relay is in Network › Connection, AWS only in Cloud credentials, both kinds of source in one Sources list.

**Agent page.** `{ kind: 'agent', agentId, tab? }` opens Overview unless `tab` says otherwise (`'chat' | 'runs' | 'settings'`); links that mean "talk to it" pass `tab: 'chat'`. Settings saves through `useSaveAgent` (the definition files); instructions and tools are edited in Studio, opened on the agent with `openAgentInStudio(agentId)` (`src/lib/ask.ts`). The list and the health card share one call, `listAgentOverviews(todayStart)`; plain-word helpers (runtime names, problems, model choices) live in `src/components/agents/agent-page-model.ts`.

**Workflow page.** `{ kind: 'workflow', workflowId, tab? }` opens Overview unless `tab` says otherwise (`'graph' | 'runs' | 'configuration'`); a workflow made from a template opens on `graph`. Plain-word rules for Automations (cron in words, priority levels, step labels, who started a run, where a run or firing opens) are pure functions in `src/components/automations/automations-model.ts`. Cron validity comes from the backend (`useCronPreview`), never from the words.

**Leave guard.** A page with an unsaved draft calls `setLeaveGuarded(dirty)` in an effect, and resets it to `false` on unmount. While guarded, `navigate`, `replace` and `goBack` park the move in `pendingLeave` (a move to the same view goes through), and `LeaveGuardDialog` (mounted in `MainPane`) asks: **Stay** calls `cancelLeave`, **Leave and discard** calls `confirmLeave`, which makes the move and clears the guard. Keep the draft in the page, not in a tab's content, so switching tabs doesn't lose it (see `WorkflowDetailView`).

There's no message box docked under every page. The agent page's Chat tab has its own composer (`AgentComposer`: Chat messages the agent, Ask asks about it), Studio has its own, and the command palette's first row ("Ask Nandi: …") and Home's ask entry send text to Studio (`sendToStudio`).

Views read state only from the agent store, which mirrors SQLite through snapshots and events. See [agent-runtime.md](./agent-runtime.md).

## File Organization

```
locales/                  # Translation JSON files
src/
├── components/
│   ├── agents/          # Agents list and agent page (tabs, dialogs, page model)
│   ├── agent-console/   # Chat, approvals and settings panels the agent page reuses
│   ├── layout/          # Layout components (MainWindow, sidebars)
│   ├── command-palette/ # Command palette system
│   ├── preferences/     # Preferences dialog system
│   └── ui/              # Shadcn UI components
├── hooks/               # Custom React hooks
├── i18n/                # Internationalization config
├── pages/               # Top-level screens rendered inside MainWindow
├── lib/
│   ├── commands/        # Command system implementation
│   └── menu.ts          # Native menu builder with i18n
├── services/            # TanStack Query + Tauri integration
├── store/               # Zustand stores
└── types/               # Shared TypeScript types
```

## Multi-Window Architecture

Tauri applications can have multiple windows, each running a separate JavaScript context. Windows cannot share React state directly.

**Key patterns:**

1. **Separate entry points** - Each window has its own HTML file and React root
2. **Event-based communication** - Use Tauri events to communicate between windows
3. **Window reuse** - Create windows once at startup, then show/hide as needed
4. **Theme synchronization** - Emit theme changes so all windows stay in sync

```typescript
// Window A: emit event
await emit('data-updated', { value: 'new data' })

// Window B: listen and react
listen('data-updated', ({ payload }) => {
  setData(payload.value)
})
```

See [companion-window.md](./companion-window.md) for the app's second window: entry points, cross-window events, and the `ViewOpener` context that lets rows shared between windows open views in the right one.

## Security Architecture

### Tauri Capabilities

Tauri v2 uses a permission-based capabilities system. Each window only gets the permissions it needs.

**Location:** `src-tauri/capabilities/default.json`

```json
{
  "identifier": "main-capability",
  "windows": ["main"],
  "permissions": ["core:window:allow-minimize", "fs:default"]
}
```

**Key rules:**

- Use specific window labels, not `["*"]`
- Only add permissions actually needed
- Remote content (if any) should have minimal permissions

### Content Security Policy

CSP prevents XSS attacks. Configuration is in `src-tauri/tauri.conf.json`.

**Rules:**

- Never load scripts from CDNs - bundle everything locally
- Avoid `'unsafe-eval'` unless absolutely necessary
- Images: restrict to specific domains when possible

### Secure Storage

| Data Type       | Storage                       | Security Level |
| --------------- | ----------------------------- | -------------- |
| API tokens/keys | OS keychain (`keyring` crate) | High           |
| App preferences | App data directory (JSON)     | Medium         |
| User content    | App data directory/SQLite     | Medium         |

Never store sensitive tokens in `tauri-plugin-store` (plain JSON on disk). See [external-apis.md](./external-apis.md) for keychain patterns.

### Rust-First Security

All file operations happen in Rust with built-in validation:

```rust
fn is_blocked_directory(path: &Path) -> bool {
    let blocked_patterns = ["/System/", "/usr/", "/etc/", "/.ssh/"];
    blocked_patterns.iter().any(|pattern| path.starts_with(pattern))
}
```

### Input Sanitization

```rust
pub fn sanitize_filename(filename: &str) -> String {
    filename.chars()
        .filter(|c| !['/', '\\', ':', '*', '?', '"', '<', '>', '|'].contains(c))
        .collect()
}
```

### Atomic File Operations

All disk writes use atomic operations to prevent corruption:

```rust
// Write to temp file, then rename (atomic)
std::fs::write(&temp_path, content)?;
std::fs::rename(&temp_path, &final_path)?;
```

See [Tauri Security Documentation](https://v2.tauri.app/security/) for detailed guidance.

## Type-Safe Tauri Commands

All Tauri commands use [tauri-specta](https://github.com/specta-rs/tauri-specta) for type safety:

```typescript
// ✅ GOOD: Type-safe with autocomplete
import { commands } from '@/lib/tauri-bindings'

const result = await commands.loadPreferences()
if (result.status === 'ok') {
  console.log(result.data.theme)
}

// ❌ BAD: String-based invoke (no type safety)
const prefs = await invoke<AppPreferences>('load_preferences')
```

See [tauri-commands.md](./tauri-commands.md) for adding new commands.

## Quality Gates

Before any changes are committed:

```bash
pnpm run check:all
```

See [static-analysis.md](./static-analysis.md) for all tools included.

## Anti-Patterns to Avoid

| Anti-Pattern                    | Why It's Bad                        | Do This Instead               |
| ------------------------------- | ----------------------------------- | ----------------------------- |
| State in wrong layer            | Confuses ownership, breaks patterns | Follow the onion model        |
| Direct Rust-React coupling      | Tight coupling, hard to maintain    | Use command system and events |
| Store subscription in callbacks | Causes render cascades              | Use `getState()` pattern      |
| Skipping input validation       | Security vulnerabilities            | Always validate in Rust       |
| Magic/implicit patterns         | Hard for AI and humans to follow    | Prefer explicit, clear code   |

## Adding New Features

1. **Commands** - Add to appropriate command group file
2. **State** - Choose appropriate layer (useState/Zustand/TanStack Query)
3. **UI** - Follow component architecture
4. **Persistence** - Use established [data-persistence.md](./data-persistence.md) patterns
5. **Testing** - Add tests following [testing.md](./testing.md) patterns
6. **Documentation** - Update relevant docs
