# Transports: Desktop IPC and Web

The same Nandi core serves two transports: Tauri IPC for the desktop webview, and HTTP + WebSocket for browsers.

```
            services/  (Nandi core — no Tauri types)
            AppState: db · orchestrators · event bus · secrets · paths
                 │                                │
       commands/ (Tauri IPC)               http/ (Axum)
       1–3 line adapters                   /api/* REST · /ws events · / frontend
                 │                                │
          Tauri webview                        Browser
                 └──────── src/lib/client ────────┘
                  tauriClient            webClient
```

## Rules

- **`services/` and `http/` never import Tauri.** Services take `&AppState`; `AppState::start(Config)` is called by both `lib.rs` (desktop) and `headless.rs`.
- **One declaration per command.** Business logic lives in a service. Declare the command once with `rpc::commands!` in `src-tauri/src/rpc/<module>.rs`: one line gives the Tauri command (desktop) and `POST /api/rpc/{name}` (web). Register it in `bindings.rs` and run `pnpm run rust:bindings`; the UI then calls `client.<camelName>` on both transports. Commands that need the `AppHandle`, a window or a typed error stay hand-written in `commands/` with a route in `http/`. Modules not yet moved to `rpc/` still use that older pair.
- **The UI never calls `invoke()`/`listen()` for data.** Use `client` from `@/lib/client`. Desktop-only chrome (windows, menus, updater, companion window, OS plugins) is gated by `isDesktop()`.
- **Paths never come from a client.** The native file picker runs in the Tauri adapter (`commands/attachments.rs`) and stages files; clients only hold attachment IDs. The web client reports attachments as unsupported.
- **Secrets** use the `keyring` crate via `services::secrets`, under the app identifier (`services::APP_IDENTIFIER`, checked against `tauri.conf.json` by a test).

## Events

`EventBus` emits to a `FanoutSink`: the Tauri webview (`commands::events::TauriSink`) and WebSocket subscribers (`BroadcastSink`). Wire format on `/ws`: `{"event": "agent://token", "payload": {…}}`.

`client.subscribe(handler)` returns `{ ready, unsubscribe }`. Load snapshots only after `ready`, so no event can slip between a snapshot read and the first listener. The web client emits `nandi://resync` after a reconnect, and the server sends it when a client lags; `agent-sync` then reloads all snapshots.

New event types: add the struct with `event!` in `services/events.rs`, mirror it in `commands/events.rs` (`tauri_events!`), register it in `bindings.rs`, and add it to `src/lib/client/events.ts`.

## Web access

| Launch                             | Web port                                                     |
| ---------------------------------- | ------------------------------------------------------------ |
| default                            | `127.0.0.1:<random>` (logged; token in `~/.nandi/web_token`) |
| `--server [--port N]`              | `0.0.0.0:17463` (or N); access link printed                  |
| `--headless [--server] [--port N]` | no window; `127.0.0.1:17463` unless `--server`               |

In dev: `pnpm run tauri dev -- -- --server`. Headless from the repo: `cargo run --manifest-path src-tauri/Cargo.toml -- --headless`.

**Auth is always required.** Open the printed link (`/?token=…`): the token is exchanged for an `HttpOnly; SameSite=Strict` cookie and stripped from the URL. Scripts can send `Authorization: Bearer <token>`. The token is created on first launch (mode 0600 on Unix).

**Frontend bundle.** The server serves `dist/` via `rust-embed`: embedded in release builds, read from disk in debug builds, so run `pnpm run build` before testing the web UI in dev. Unknown paths fall back to `index.html`.

Release builds on Windows have no console of their own: when started from a terminal with flags, Nandi attaches to that terminal's console, and `--server` / `--headless` also write the access link to `~/.nandi/server.log`. `--help` lists the flags; unknown flags are rejected.

## REST routes

`POST /api/rpc/{name}` runs any command declared with `rpc::commands!`. The JSON body is its arguments with camelCase keys, as over IPC: `POST /api/rpc/request_for_run {"runId": "r1"}`. Unknown names are `404`. The generated `commands` reach it through `src/lib/client/invoke.ts`.

The hand-written routes: `/api/agents` (GET, POST) · `/api/agents/{id}` (DELETE) · `/api/agents/{id}/start|stop|restart|messages|approve|cancel` (POST) · `/api/agents/{id}/queue|current` (GET) · `/api/queue/{runId}/cancel` (POST) · `/api/queue/{runId}/priority` (POST `{ priority }`) · `/api/triggers…` (see [Triggers](./triggers.md#api)) · `/api/snapshots/agent/{id}` · `/api/snapshots/dashboard?dayStart=` · `/api/runs/{id}` · `/api/workflows` · `/api/studio/conversations` (GET, POST) · `/api/studio/conversations/{id}/messages` (POST) · `/api/conversations/{id}` · `/api/attachments/{id}` (DELETE) · `/api/preferences` (GET, PUT) · `/api/llm-keys` (GET) · `/api/llm-keys/{provider}` (PUT) · `/api/emergency-data/{name}` (GET, PUT) · `/api/emergency-data/cleanup` (POST).

Errors are `400 {"error": "…"}` with the same messages the IPC commands return; auth failures are `401`.

## Agent lifecycle

`create_agent` writes `~/.nandi/repo/.nandi/agents/<id>/skill.md` (staged in git) and starts its orchestrator. `start_agent` / `stop_agent` / `restart_agent` manage its runtime container. `delete_agent` refuses during an active run or while requests are queued, stops the container and orchestrator, moves the folder to `~/.nandi/trash/` (run history stays), and is refused while a workflow or trigger uses the agent. Both emit `repo://agent-created`/`deleted` and `agent://agents_changed`.

The agent snapshot's `container` (`idle` / `starting` / `running` / `error`, plus the last start error) comes from `AgentRuntimes`; every status change also emits `agent://agents_changed`, so clients reload the snapshot. The Agent view's header has Start / Stop / Restart buttons over these commands. A container stopped outside Nandi is reconciled to `idle` within 15s, or immediately when the snapshot is loaded (see [Agent Runtime](./agent-runtime.md)).

If the container can't start when a run needs tools, the step runs **text-only** (no tools offered, and the system prompt says so) and the core emits `agent://warning` `{ agentId, message: "Tool execution unavailable: …" }`. The UI shows it as a banner until it's dismissed or the container is running. Warnings aren't recorded.
