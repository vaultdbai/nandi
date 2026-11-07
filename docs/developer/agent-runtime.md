# Agent Runtime

Rust is the source of truth; React mirrors it. The core lives in `src-tauri/src/services/` and is served over both Tauri IPC and HTTP/WebSocket; see [transports.md](./transports.md). Each agent has a long-lived orchestrator task. Agent definitions are files in the gold copy (`~/.nandi/repo`, see [data-model.md](./data-model.md)); runtime state (runs, messages, approvals, queue) lives in SQLite at `~/.nandi/local/state.db`, and the UI rebuilds everything from snapshots plus event deltas. Navigation never affects execution.

```
React (agent-store; listeners registered once at boot)
  ├─ snapshots: get_agent_snapshot · get_dashboard_snapshot · get_conversation · get_run_detail · …
  └─ commands:  send_message · approve_action · cancel_turn ──▶ command (DB txn) ──▶ mpsc
                                                                                        ▼
                                          Orchestrator task per agent (+ "@studio"), app lifetime
                                                       │ executor: abortable child task per run
                                                       ▼
                                 events.rs: write SQLite (one txn) THEN emit agent://* · dashboard://*
```

## Rules

- **No state in React that Rust can't reproduce.** The agent store (`src/store/agent-store.ts`) holds snapshots plus deltas, nothing else. The one exception is the optimistic user message, which uses a client ID that Rust persists before `send_message` returns. If the reply says `queued`, the UI drops it again: Rust writes the message (same ID) when the run starts.
- **Write before emit.** Every `EventBus` method commits its transaction, then emits. A missed event is never lost; the next snapshot has it.
- **Events are idempotent.** Messages are upserted by ID; tokens carry the UTF-16 offset they were appended at. Reducers ignore duplicates and report a gap (→ snapshot reload) instead of guessing.
- **Event names are global** (`agent://token`, not per agent), with `agentId`/`conversationId` in the payload: the frontend registers listeners once and Tauri has no wildcard listeners.
- **Attachments are picked in Rust.** `pick_attachments` opens the dialog and stages files; the webview only holds IDs. Never accept file paths from the webview.
- **Specta field names:** avoid digits in snake_case fields (`failed_last_24h`). Serde and specta camelCase them differently, so the TS type won't match the JSON.
- **Keys are write-only from the UI**, agent IDs are paths (validate with `agents::validate_agent_id`), and agent output is untrusted (`MarkdownContent` renders no HTML or images).

## One turn per agent

Each orchestrator runs one turn at a time. Its `current` field (the run ID of the running or approval-blocked turn) is what blocks a second turn, not the channel. A `SendMessage` that arrives while `current` is set, or while others are already waiting, becomes a `queued` run plus an `agent_queue` row holding its input. Whenever the turn ends (completed, failed, or cancelled), the orchestrator pops the next row (lowest `priority`, then oldest), writes its user message, and starts it. A run suspended for approval is still the current turn, so the queue waits until it's resolved or stopped.

| Source                      | Priority |
| --------------------------- | -------- |
| User (`PRIORITY_USER`)      | 10       |
| Critical workflow step      | 30       |
| Workflow step               | 50       |
| Agent delegation (A2A)      | 60       |
| Scheduled                   | 80       |
| Background (`runs` default) | 100      |

Triggers add `cron` (80), `file` (70), `webhook` (60), `device` (50), and `event` (60) sources by default; see [Triggers](./triggers.md).

- `Approve` and `Cancel` only act on the current turn. `cancel_queued_run` / `reprioritize_queued_run` change the queue; they're orchestrator commands too, so they're ordered with the turns.
- Every enqueue, start, cancel, reprioritize, and turn end emits `agent://queue_changed` `{ agentId, queue, current }` from the same transaction. It carries the whole state, so the latest event wins and the UI needs no polling.
- The queue is in SQLite, so it survives restarts: on startup an orchestrator takes any approval-blocked run as its current turn, then drains the queue in order.

## Run state machine

A run is persisted, so it can suspend for approval (even across restarts) and resume:

```
queued ─▶ (agent free) ─▶ running ─▶ plan step (assistant message + planned calls) ─▶ stream text ─▶ each planned call:
  needs approval, none yet ─▶ approval row + message ─▶ awaiting_approval   (suspend; task stays free)
  approved / no approval   ─▶ tool_call ─▶ execute ─▶ tool_result
  rejected / expired       ─▶ skipped
no more steps ─▶ completed ─▶ turn_done
approve_action ─▶ resolve txn ─▶ resume     cancel_turn ─▶ abort child, expire approvals ─▶ turn_done(cancelled)
expires_at passed (sweep, every 60 s) ─▶ ExpireApproval ─▶ close as expired ─▶ resume
```

Every tool approval gets `expires_at = created + APPROVAL_TTL_MS` (one hour, `model.rs`; v16 backfilled pending ones). `AppState::start` runs `chat::expire_overdue_approvals` once a minute: each overdue approval is sent to its orchestrator as `OrchestratorCommand::ExpireApproval`, which closes it through the same path as a decision (`EventBus::approval_closed`, status `expired`, `agent://approval_resolved`) and resumes the run; the model reads the expiry when it replans. Escalated clarification questions (`delegation_question`) have no `expires_at`: their clarification's deadline governs them. `list_pending_approvals` feeds the Inbox (`src/components/inbox`).

On startup, runs left `running` are marked failed (their step died with the process); runs `awaiting_approval` survive, and each orchestrator re-announces its pending approvals. A panicking run or orchestrator is logged, the agent is marked `error` in `agent_state`, and the orchestrator restarts (up to 5 times).

## Engine

Runs are planned by a real model by default (`settings.engine` absent or `"llm"`). Each step, `services/orchestrator.rs`:

1. loads skill.md (+ optional `instructions.md`) for the system prompt and model (`model:` frontmatter; default: the settings default model, `claude:claude-sonnet-5` out of the box). `llm::stream` calls that model when its provider has a key in the keychain, else the first provider with a key (Anthropic, OpenAI, DeepSeek; `llm_keys::usable_model`);
2. for agents, gets the runtime container + MCP session (`services/runtimes.rs`, started on first use; needs Docker and the `nandi-runtime` image) and offers its tools, filtered by `allowed-tools` and its policies (`RepoStore::skill` → `services::policies::narrow`);
3. rebuilds the history from the DB (`services/history.rs`): every tool call is answered (output, "rejected", or "not run"), and DeepSeek `reasoning_content` is replayed (required with tools);
4. streams the reply (`llm::stream`), batching tokens into ~50 ms DB writes + `agent://token` events, then records the planned calls and reasoning on the assistant message;
5. runs the calls (approval-gated per `requires_approval`) and loops. A step with no tool calls completes the run.

Studio uses the same loop with no tools and its own system prompt.

**DeepSeek** (`llm/deepseek.rs`): streamed Chat Completions with `stream_options.include_usage`. 429/500/503 retry twice before any output; 401/402/429 map to user-facing messages. Cost uses DeepSeek's peak/off-peak UTC windows (holidays not modeled) and is written to `~/.nandi/local/logs/llm.log` with token counts.

**Keys**: Settings → Models (`set_llm_api_key`, provider `deepSeek`) stores the key in the OS keychain as `nandi.llm.deepseek`; never in the DB or config.

**Cancel** aborts the run's child task, which drops the in-flight stream and closes the HTTP connection.

**Canned engine**: set `settings.engine` to `"canned"` to replay the `canned_script` setting instead of calling a model (tests use this):

```sql
INSERT INTO settings (key, value) VALUES ('engine', '"canned"')
  ON CONFLICT(key) DO UPDATE SET value = excluded.value;
```

## Turn middleware

Each engine step of a run (one LLM call and its tool calls) is wrapped by a pipeline of `TurnMiddleware`s (`services/orchestrator/middleware.rs`). A "turn" in this API is one step: `TurnContext::turn_number` is the 0-based step index.

```
execute_run: policy = TurnPolicy::from_extra(agent.extra()); pipeline = register_default_middlewares(deps, &policy)
loop:
  run the last step's planned calls  (a call some middleware's allows_tool() rejects gets an error result, no approval)
  last step text-only, no continuation after it ─▶ completed
  refresh TurnContext from the DB (cost, token estimate, tool results, files touched; system_extras cleared)
  pre_turn × N, in order ─▶ Continue | Abort(reason) | PauseForApproval(reason) | ctx.run_id replaced
  engine step (llm_step appends ctx.system_extras to the system prompt)
  post_turn × N, in reverse ─▶ continue_with? append it and loop : (text-only? completed)
```

**Pipeline order** (`register_default_middlewares`): `turn_limit`, `cost_cap`, `token_cap` (always), `memory` (unless `memory_disabled`), `compaction` and `verify_on_stop` (when enabled), `read_only` (when declared). Studio, and agent IDs with no definition, keep only the three built-in limits, since the others work on an agent folder.

**skill.md keys** (unknown frontmatter, read from `Agent::extra()`; an invalid value logs a warning and takes its default):

| Key               | Default                                                                        | Notes                                                       |
| ----------------- | ------------------------------------------------------------------------------ | ----------------------------------------------------------- |
| `middlewares`     | not declared: `verify_on_stop`, `compaction`                                   | `[]` disables both; `[read_only]` is plan mode              |
| `max_turns`       | 50                                                                             | Steps per run (a positive integer)                          |
| `max_cost_usd`    | 5.0                                                                            | Stops once the run's summed `cost_usd` passes it            |
| `compaction`      | `{ trigger_tokens: null, preserve_recent_percent: 30, iterative_merge: true }` | Each key falls back on its own; `null`: window minus buffer |
| `memory_disabled` | false                                                                          |                                                             |

**Decisions.**

- `Abort(reason)`: the run ends `failed` with `reason` as its error message (shown in the chat and as a dashboard attention item), then `agent://turn_aborted` `{ agentId, runId, middleware, reason }` is emitted. The child task returns `Ok`, so the queue moves on as after any finished turn.
- `PauseForApproval(reason)`: the run suspends through the normal approval flow, on a synthetic call (tool `continue_run`, args `{ middleware, reason }`, call ID `pause:<run>:<step>:<middleware>`). Approving resumes the run, and that middleware's pause counts as `Continue` for that step; rejecting ends the run as an abort ("… (not approved)").
- `continue_with` (set in `post_turn`): appended to the run as a **user-role** message (the role the model history replays; `continuation: true` in its metadata), and the loop takes another step instead of completing.
- `ExitReason::Aborted` set in `post_turn` aborts like a `pre_turn` abort.
- A middleware that returns `Err` is logged and skipped, so a broken optional middleware never fails a run.

**Context fields.** `accumulated_cost` is the run's `cost_usd`; `token_estimate` is chars / 4 over the run's messages (content, plus metadata for non-user rows); `context_window` comes from the agent's model (`context_window_for`: 128k for DeepSeek, else 200k); `last_tool_results` and `files_touched` are rebuilt from the run's finished tool calls (`files_touched`: the `path` / `file_path` of successful write-like calls). `TurnResult::files_modified` holds the write paths the step _planned_: `post_turn` runs right after the step is planned, before its calls run. The context lives for one `execute_run` call: a run resumed after an approval rebuilds it from the DB, so middleware-private flags such as `verify_fired` reset across a suspension.

**Plan mode** (`read_only`) blocks `write_file`, `edit_file`, `create_file`, `append_file`, `delete_file`, `remove_file`, `move_file`, `rename_file`, `copy_file`, `make_dir`, `apply_patch`, `shell`, and `run_command`, matched after any connector prefix (`filesystem__write_file`). A blocked call is recorded with the error result "Blocked by plan mode (read-only): …" so the model reads why; it's never offered for approval.

**Limits.** `turn_limit` aborts before step `max_turns` (a run takes at most `max_turns` steps; this replaced the old fixed 25-step cap), `cost_cap` once the cost is over `max_cost_usd`, and `token_cap` once the estimate is over the context window (a hard stop; compaction normally starts a child run well before).

**Compaction seam.** A `pre_turn` that replaces `ctx.run_id` (compaction's child run) makes the child task return `RunEnd::ContinuedIn(new_id)`. The orchestrator moves its current turn to the child run (so the queue keeps waiting and Approve/Cancel target the child) and starts executing it. The middleware must persist the child run as `running` and end the parent itself (e.g. as `compacted`).

## Schema

`conversations`, `messages` (+ `run_id`), `runs` (+ summed LLM usage: `input_tokens`, `output_tokens`, `cost_usd`, `model`; + source: `source_type`, `source_id`, `source_label`, `priority`, `queued_at`, `queue_position`), `agent_queue` (waiting runs and their input), `run_events` (every non-token event, for the Activity tab and run detail), `approvals` (+ `message_id`, `tool_call_id`), `workflows`, `workflow_runs`, `settings`, `agent_state`. JSON columns are `TEXT`: a column declared `JSON` gets NUMERIC affinity and would coerce values like `"123"`.

Each agent has one conversation, `agent:<id>`. Studio conversations belong to the pseudo-agent `@studio`.

## Frontend

- `src/lib/agent-sync.ts`: subscribes via `client.subscribe()` (IPC or WebSocket), then loads snapshots. Events for a conversation that is mid-load are buffered and replayed on top of the snapshot. Dashboard aggregates are refreshed from the DB when a run starts or ends.
- `src/lib/store-reducers.ts`: pure, idempotent reducers (unit-tested).
- `src/lib/agent-actions.ts`: user actions (`sendAgentMessage`, `decideApproval`, `cancelTurn`, `cancelQueuedRun`, `reprioritizeQueuedRun`, Studio).
- `src/components/agent-console/TurnQueue.tsx`: the strip above an agent's chat (current turn, "+N queued") and the queue panel. Dragging a row calls `priorityForMove` (`src/lib/queue-order.ts`) and reprioritizes.
- Selectors: `useAgent`, `useMessages`, `useCurrentRun`, `useCurrentTurn`, `useAgentQueue`, `usePendingApprovals`, `useDashboard`, `useConversation`, `useStudioConversations`.

## Container (for the LLM engine)

Image: pulled from GitHub's container registry, or built here (`docker build -t ghcr.io/vaultdbai/nandi-runtime:sandbox-6 runtimes/nandi-runtime`); see [Harness Container › Get the images](./harness-container.md#get-the-images). The agent folder mounts read-only at `/agent/skill` and `~/.nandi/local/work/<id>` read-write at `/agent/workdir`. Hardening: user `1000:1000` set by the app, read-only root filesystem (tmpfs `/tmp` and `/home/agent`), `cap_drop: ALL`, `no-new-privileges`, memory/PID limits, an image labelled `org.nandi.sandbox` at `runtime::SANDBOX_VERSION` or later, only the internal `nandi-agents` network and no published ports. The host runs the tool server per session with `docker exec -i <container> python /opt/nandi/app.py --stdio` (`mcp_client.rs`); the container's own process just idles. `cargo test runtime_e2e -- --ignored --nocapture` exercises it against real Docker. Network, AWS, and LLM access go through the host egress proxy; see [Egress Proxy](./egress.md).

**Shared folders.** The filesystem connector's config lists folders (`folders: [{id, path, access: read|write}]`). An agent names the ones it gets in skill.md (`connectors: [{id: filesystem, folders: [docs, {id: reports, access: read}]}]`), and can narrow access but never widen it. Without `folders` it gets none, so a folder added later is never given to an agent silently. Its policies may then drop a folder or make it read-only (`services::policies`). The same set reaches the agent three ways, all from `connectors::agent_folders` / `folders::granted`: the connector's tools (`call_tool_for` gets the agent's item; paths are `<folder>/<path>`), bind mounts at `/agent/folders/<id>` (`:ro` unless write) in the native container and in CLI runtime containers (`Placement::Container.folders`), and a line per folder in the system prompt (`folders::prompt`). Folder IDs that aren't on the list fail `nandi_check_artifact` and `set_agent_connectors`.

**Text-only fallback.** If the container can't start when a run needs tools, the step runs text-only (no tools offered, and the system prompt says so) and the core emits `agent://warning` `{ agentId, message: "Tool execution unavailable: …" }`. The UI shows it as a banner until it's dismissed or the container is running. Warnings aren't recorded. See [Transports](./transports.md).

**Status reconciliation.** Container status is held in memory by `AgentRuntimes`. A container stopped outside Nandi (`docker stop`, a crash) is detected by `AgentRuntimes::reconcile`, which inspects it via Bollard on every `get_agent_snapshot` and from a background loop every 15s while any container is `running`. The loop exits when none are, so an idle app doesn't poll. On a mismatch the session is dropped, the status becomes `idle`, and `agent://agents_changed` is emitted; the next tool call starts a fresh container.

## Testing

- `cargo test`: DB queries, write-before-emit, and the full state machine (suspend/approve/reject/cancel/restart/Studio) with a recording event sink.
- `pnpm exec vitest run`: reducers, sync buffering and resync, optimistic send, and shell flows driven by mock events (`src/test/mock-events.ts`).
