# E2E Checklist: Full Agent Loop

A manual test that runs one full turn with a real DeepSeek model and a real runtime container: user message → `shell` tool call → result → final answer, shown live in the UI and persisted to SQLite.

## Prerequisites

1. Get the runtime image: Settings › Models & runtimes › Download (or `docker build -t ghcr.io/vaultdbai/nandi-runtime:sandbox-6 runtimes/nandi-runtime`)
2. Add a DeepSeek key in Nandi under **Settings → Models**. Launch Nandi once so `~/.nandi/local/state.db` exists and is migrated.
3. Install the fixture **before launching Nandi** (agents found on disk get an orchestrator at startup):

   ```powershell
   New-Item -ItemType Directory -Force "$HOME\.nandi\repo\.nandi\agents\echo-agent"
   Copy-Item test-fixtures\echo-agent\skill.md "$HOME\.nandi\repo\.nandi\agents\echo-agent\skill.md"
   ```

4. Check prerequisites:

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts/e2e-smoke.ps1
   cargo run --manifest-path src-tauri/Cargo.toml -- verify --agent echo-agent
   ```

   `verify` is a command of the app binary (`nandi`), not a separate CLI. It checks skill.md, the `shell` permission, Docker, the image, the DB schema, and the model's API key. Both exit non-zero on failure.

The image's own process just idles (`sleep infinity`); Nandi starts the tool server per session with `docker exec -i … python /opt/nandi/app.py --stdio`. `requires_approval: []` lets `shell` run without an approval prompt. For the sandbox itself (network, approvals, policies) see the [Sandbox and Policy Test Checklist](./sandbox-test-checklist.md).

## Logs to tail

| What                          | Where                                                                                                                                                                                                                                                            |
| ----------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| LLM calls (one line per call) | `~/.nandi/local/logs/llm.log`: JSON lines with `model`, `status`, tokens, `duration_ms`, `cost_usd`, `error`. Metadata only, no prompt text. `Get-Content $HOME\.nandi\local\logs\llm.log -Wait -Tail 20`                                                        |
| Container egress              | `~/.nandi/local/logs/egress.log`. Expect nothing: this test makes no network calls from the container                                                                                                                                                            |
| Tool server                   | Runs over `docker exec` stdio, so `docker logs` shows nothing; watch tool calls in the app log and the run's Logs tab                                                                                                                                            |
| Nandi app log                 | `pnpm run tauri:dev` terminal (stdout, debug level). Windows release builds have no file log: run a dev build. Look for `Started runtime container nandi-agent-<tag>-echo-agent` (`<tag>`: eight hex characters for this Nandi) and `runtime ready with 4 tools` |
| SQLite                        | `~/.nandi/local/state.db` (WAL mode). Tables `runs`, `messages`, `run_events`                                                                                                                                                                                    |

## Steps

Prompt to send: `Run "echo hello from nandi && uname -a" and tell me the output.`

| #   | Step                                         | Expected                                                                                                                                    | Pass |
| --- | -------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- | ---- |
| 1   | Agent exists (skill.md, image, DeepSeek)     | **Echo Agent** in the sidebar; agent view loads with no skill.md error                                                                      | ☐    |
| 2   | Container starts                             | Start (or the first message) → status `starting` → `running`; `docker ps` shows `nandi-agent-<tag>-echo-agent`; no `agent://warning` banner | ☐    |
| 3   | Send a message that needs `shell`            | User message appears immediately; a run starts                                                                                              | ☐    |
| 4   | DeepSeek plans and emits `tool_call`         | Tokens stream; a `shell` tool call card appears with `cmd`; one `ok` line in `llm.log`                                                      | ☐    |
| 5   | Orchestrator dispatches to the MCP container | `POST /mcp` lines in `docker logs`; no approval prompt                                                                                      | ☐    |
| 6   | Container executes and returns the result    | Tool result shows `stdout` containing `hello from nandi`, `exit_code: 0`                                                                    | ☐    |
| 7   | Result goes back to DeepSeek                 | A second `ok` line in `llm.log` for the same agent                                                                                          | ☐    |
| 8   | DeepSeek produces the final answer           | Assistant message quotes the output; run ends `completed`                                                                                   | ☐    |
| 9   | Live in the UI and persisted to SQLite       | Everything above appeared without a reload. After restarting Nandi, the conversation, tool call, and result are still there                 | ☐    |
| 10  | Run in RunHistoryView with usage stats       | Run listed with token counts and cost (cost shows only when the model has a known price)                                                    | ☐    |
| 11  | Dashboard updates in real time               | With the dashboard open during a second run: running count goes up, then down; activity and recent outputs update without a reload          | ☐    |

Optional negative check: `docker stop nandi-agent-<tag>-echo-agent`. Within 15s, or right away when the agent view reloads, status goes to `idle`. The next message starts a fresh container.

## Observed vs expected

Fill in as you run. Note the run ID (RunHistoryView) and the matching `llm.log` timestamps.

| #   | Observed | Matches? | Notes / log excerpt |
| --- | -------- | -------- | ------------------- |
| 1   |          |          |                     |
| 2   |          |          |                     |
| 3   |          |          |                     |
| 4   |          |          |                     |
| 5   |          |          |                     |
| 6   |          |          |                     |
| 7   |          |          |                     |
| 8   |          |          |                     |
| 9   |          |          |                     |
| 10  |          |          |                     |
| 11  |          |          |                     |

Environment: Nandi commit `_____` · OS `_____` · Docker `_____` · model `deepseek:deepseek-flash`
