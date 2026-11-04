# Companion Window

The companion is Nandi's second window: an always-available orb that expands to a pocket Home and Studio. It's the app's multi-window example. It replaces the template's quick pane, which was removed in task 16.

## Entry points

Each Tauri window runs its own JavaScript context and React root. Windows can't share React state or Zustand stores.

```
index.html      → src/main.tsx            → the main app
companion.html  → src/companion-main.tsx  → CompanionWindow (own QueryClient, theme, language)
```

`vite.config.ts` builds both inputs. The window (label `companion`) is created once at startup and shown or hidden after that (`src-tauri/src/commands/companion.rs` `setup`). Its permissions are in `capabilities/companion.json`. **Ctrl/Cmd+Shift+Space** toggles it from anywhere (`COMPANION_SHORTCUT`).

In the web build there's no second window. `MainWindow` renders `<CompanionApp docked />` in the corner, with the same content.

## Sharing data, not state

The companion reads the same TanStack queries as Home (`useHomeFeed`, `useInbox`, `useRequest`). Each webview has its own cache, and both stay live through the same backend events. So the orb's count is always the Home badge's. Don't read the agent store in shared hooks: the companion's webview doesn't sync it, so use `useAgents()` instead.

## Opening views: `ViewOpener`

Rows shared with Home (`InboxRow`, `RunningRow`, `RecentRow`, `RequestCard`) never navigate directly. They call `useViewOpener()` (`src/hooks/useViewOpener.ts`):

```tsx
const { open, openSettings } = useViewOpener()
open({ kind: 'request', requestId })
```

The default opens views in the current window. The companion provides `<ViewOpenerContext value={opener}>`:

- standalone: `openInMain(view)` / `openSettingsInMain(pane)` focus the main window and emit an event;
- docked (web): navigate directly.

## Cross-window events

| Event                       | From → to           | Payload         | Effect                                             |
| --------------------------- | ------------------- | --------------- | -------------------------------------------------- |
| `companion://navigate`      | companion → main    | `View`          | The main window navigates                          |
| `companion://open_settings` | companion → main    | `{ pane }`      | The main window opens Settings at that pane        |
| `companion://focus_request` | backend → companion | `{ requestId }` | The card opens with that Request pinned in Running |

The constants are in `src/lib/companion-events.ts` and `FOCUS_REQUEST_EVENT` in `commands/companion.rs`. **Watch in companion** calls `companion_focus_request(id)`, which shows the window and emits only to it (`emit_to(COMPANION_LABEL, …)`).

## Notifications

`useCompanionNotifications` runs the pure `noticesFor` (`companion-model.ts`) on every Inbox change. The rules:

- notify only about items that are new since the last look, and only while the card is hidden;
- never notify about what was there at launch, and never about failures;
- warn once, 5 minutes before an item expires.

Sends go through `commands.sendNativeNotification`. The notification plugin has no action buttons.

## The conversation

`send_companion_message(text, conversationId, scopeRequestId)` sends to the companion's Studio conversation, which is titled "Companion". An "@Agent …" prefix routes the message to that agent's chat instead. The scope is stored as a setting and turned into a prompt section by `studio::prompt` (`companion_section`), so no hidden footer is added to messages. `MarkdownLinkContext` (`agent-console/markdown-link-context.ts`) lets the companion render `nandi://request/<id>` links as live `RequestCard`s.
