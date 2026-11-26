# Agents, agent console (chat) and runs

Scope: the Agents list and agent page (all tabs and dialogs), the agent.md format the app honors, agent lifecycle, the per-agent chat (composer, messages, tool calls, approvals, queue), orchestrator behavior (turn middleware, memory, compaction, verify-on-stop, tool search), delegation surfaces shown in the console, run history and Run Detail (all tabs, analysis), the audit log viewer, and the related Tauri commands, HTTP routes and `agent://*` events.

Shared preconditions used below:

- **App running**: the desktop app (`pnpm run tauri:dev` or a build), or the headless server for `api` tests. HTTP routes are under `/api` (`src-tauri/src/http/mod.rs`), authenticated with the server token.
- **Model key**: a key for Anthropic, OpenAI or DeepSeek in Settings › Models & runtimes (OS keychain), or Ollama running, or the free trial. Without one, use the canned engine (`settings.engine = "canned"`, see `docs/developer/agent-runtime.md`).
- **Docker + image**: Docker running and `ghcr.io/vaultdbai/nandi-runtime:sandbox-6` built from Settings › Models & runtimes → Tool sandbox → Build (RT-02; the image must carry `org.nandi.sandbox=5`), needed for any tool call. CLI-runtime agents also need `ghcr.io/vaultdbai/nandi-harness:sandbox-6` (Coding agents' sandbox).
- **Agent**: an agent folder `~/.nandi/repo/.nandi/agents/<id>/agent.md`. The fixture `test-fixtures/echo-agent/agent.md` (`allowed shell`, `requires_approval: []`) is used in `docs/developer/e2e-checklist.md`.

## Agents list

### AG-01 — Agents list page

- **What:** Every agent as one row: avatar, name, live status pill, description (or "Its file can't be loaded right now."), runtime and model (or "In a container" for a CLI runtime), 7-day health cells with run counts, a problem line or "Last run …", the Shared column (`From <publisher>`, Relay, or "—") and Accepts requests On/Off. Rows waiting on the user come first, then rows with a problem, then by latest run, then by name.
- **Where:** Sidebar → Agents. `src/components/views/AgentsListView.tsx`, ordering and labels in `src/components/agents/agent-page-model.ts` (`sortAgents`, `sharedLabel`, `runtimeDetail`, `runtimeName`). Data: `list_agents` and `list_agent_overviews` (`src-tauri/src/rpc/agents.rs`, `src-tauri/src/services/agent_overview.rs`). HTTP `POST /api/rpc/list_agents`, `POST /api/rpc/list_agent_overviews` `{todayStart}`.
- **Preconditions:** At least two agents; one with recent runs.
- **Steps:**
  1. Open Sidebar → Agents.
  2. Give one agent a pending approval (send it a message that calls a tool in its `requires_approval`).
  3. Reload the list.
- **Expected:** One row per agent folder. The agent with the pending approval is first and shows "N waiting for you" (warning colour). Runtime column says "Nandi" for native agents with the model label (or "Default model"); "Claude Code"/"Codex"/"Gemini CLI" with "In a container" for CLI runtimes. Header has **Browse Library** and **New agent**.
- **Edge / negative cases:**
  - No agents: "No agents yet." empty state with **Install a starter pack** and **Describe one in Ask**.
  - `list_agents` fails: "Couldn't load agents." with **Try again**.
  - Docker stopped: list still loads; every agent shows idle (`running_agent_ids` treats Docker errors as "no containers").
  - Right-click a row opens the same menu as its "⋯".
- **Automation:** Covered by: `src/components/agents/agent-page-model.test.ts` (sorting, labels, runtime detail), Rust `#[cfg(test)]` in `src-tauri/src/services/agent_overview.rs` | Layer: api (overviews), ui-e2e (list) | Gap: no component test for `AgentsListView.tsx`.
- **Needs relay:** no
- **Status:** Shipped

### AG-02 — Agents list search and filters

- **What:** Filter rows by text ("Search agents…", name or description), runtime ("Runtime: all" or each runtime present) and an "Accepts requests" checkbox.
- **Where:** Sidebar → Agents filter bar. `src/components/views/AgentsListView.tsx`.
- **Preconditions:** Several agents, at least one with `accepts_delegation: true` and one on a CLI runtime.
- **Steps:**
  1. Type part of an agent description in Search.
  2. Choose a runtime in the runtime select.
  3. Tick Accepts requests.
- **Expected:** Rows narrow with each filter. With no match: "No agents match these filters." with **Clear filters**, which resets all three.
- **Edge / negative cases:**
  - Search is case-insensitive and trims whitespace.
  - The filter bar is hidden when there are no agents.
  - Filters reset on navigating away (component state only).
- **Automation:** Covered by: none | Layer: ui-e2e | Gap: no test.
- **Needs relay:** no
- **Status:** Shipped

### AG-03 — Agent problem line and fixes

- **What:** Each agent shows at most one problem, in this order: "N waiting for you"; agent.md model is invalid ("The model in this agent's file isn't valid."); file can't be loaded ("This agent's file can't be loaded."); CLI runtime can't sign in in its container ("Can't sign in: add an API key"); no model can be called ("No model: add one").
- **Where:** Agents list rows and Overview tab health card. `agentProblem`, `hasModelFormatError`, `runtimeAuthMissing` in `src/components/agents/agent-page-model.ts`.
- **Preconditions:** Ability to edit agent.md files and remove model keys.
- **Steps:**
  1. Set `model: claude-sonnet` (no provider prefix) in an agent's agent.md.
  2. Remove all model keys (and stop Ollama, turn off the trial).
  3. Set an agent to `runtime: claude-code` with no Claude sign-in or key.
- **Expected:** Step 1: "The model in this agent's file isn't valid." in red; the agent row status is error. Step 2: native agents show "No model: add one"; Overview offers **Connect a model**, which opens Settings › Models & runtimes. Step 3: "Can't sign in: add an API key".
- **Edge / negative cases:**
  - A pending approval outranks every other problem.
  - A CLI-runtime agent never shows noModel.
  - Signed in to Claude Code on the host with an expired session: the problem shows; once Claude Code renews it (using it on the host, or Nandi's own Claude calls), the problem clears at the next health check (within five minutes) or when Overview is opened again (`a_renewed_host_sign_in_is_reported_at_the_next_check` in `services/harness/mod.rs`).
- **Automation:** Covered by: `src/components/agents/agent-page-model.test.ts` | Layer: unit | Gap: none for the rules; UI rendering untested.
- **Needs relay:** no
- **Status:** Shipped

### AG-04 — Agent row menu

- **What:** The always-visible "⋯" on each row (or right-click) offers Open, Chat, View runs, Edit in Ask (own agents only), "Callable over the relay" (a toggle; asks first), "Sharing settings…", and "Delete agent…".
- **Where:** `src/components/sharing/AgentMenu.tsx`, used by `AgentsListView.tsx`.
- **Preconditions:** One local agent and one installed (marketplace) agent.
- **Steps:**
  1. Open "⋯" on a local agent and pick View runs.
  2. Open "⋯" on an installed agent.
- **Expected:** View runs opens the agent page on the Runs tab. On an installed agent the menu says "Installed from the marketplace: only its owner can share it", sharing items are disabled and Edit in Ask is absent.
- **Edge / negative cases:**
  - Relay sharing is covered in the network/sharing section.
- **Automation:** Covered by: `src/components/sharing/AgentMenu.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### AG-05 — New agent dialog (template)

- **What:** Creates an agent from a Name, a "What it does" (required: it becomes the `description`, which is all other agents see of it until it runs) and a "Start from" template (Blank, Researcher, Writer, Helper). The folder ID is derived from the name (lowercase letters, digits, hyphens, max 60 chars, `-2`, `-3` suffix if taken). The agent uses the default model, then its page opens.
- **Where:** Sidebar → Agents → **New agent**. `src/components/agents/NewAgentDialog.tsx`, `agentIdFrom` in `agent-page-model.ts`; command `create_agent` (`src-tauri/src/rpc/agents.rs` → `services/agents.rs::create_agent`); HTTP `POST /api/rpc/create_agent` with `input: CreateAgentInput { id, name, description, model?, instructions? }`.
- **Preconditions:** None.
- **Steps:**
  1. Click New agent, enter name "Weekly Report", pick Researcher, click Create agent.
  2. Repeat with the same name.
- **Expected:** Toast "Created Weekly Report"; the agent page opens. `~/.nandi/repo/.nandi/agents/weekly-report/agent.md` exists with `name: Weekly Report`, the description, and the Researcher instructions as body; the file is staged in git; `repo://agent-created` fires and the orchestrator starts. Second create gets ID `weekly-report-2`.
- **Edge / negative cases:**
  - Create agent is disabled with an empty name; the server also refuses ("Agent name is required").
  - Create agent is disabled with an empty "What it does"; the server, Studio's save of a new agent and Publish also refuse it ("description is required: say what the agent is for and when to use it").
  - Name over 200 chars is cut by the input; instructions over 200,000 chars are refused.
  - An ID that exists (via API) fails; an invalid model string fails with the model format error.
  - A name with only symbols gets ID `agent`.
- **Automation:** Covered by: `agent-page-model.test.ts` (`agentIdFrom`), Rust tests in `src-tauri/src/services/agents.rs` (`renders_a_parseable_skill_file`: a blank description is refused) | Layer: api | Gap: dialog itself untested.
- **Needs relay:** no
- **Status:** Shipped

### AG-06 — New agent: describe it in Ask

- **What:** The dialog's **Describe it in Ask instead** button sends "Create an agent that …" (from the description or name) to an Ask chat, or opens an empty Ask chat if both are blank.
- **Where:** `NewAgentDialog.tsx` (`sendToStudio`). Also the list's empty state.
- **Preconditions:** A model key (for Ask to answer).
- **Steps:**
  1. New agent → type a description → Describe it in Ask instead.
- **Expected:** The dialog closes and Ask opens with the prompt sent. Drafting and saving the agent are covered in the Ask section.
- **Edge / negative cases:** Both fields empty: Ask opens with no message.
- **Automation:** Covered by: none | Layer: ui-e2e | Gap: no test.
- **Needs relay:** no
- **Status:** Shipped

### AG-07 — Agent created or changed by editing files

- **What:** Writing or editing `agent.md` (and optional `instructions.md`) in the repo is picked up live by the repo watcher: the agent appears or updates within about a second. An invalid file keeps the last good version running and reports `repo://validation_failed`.
- **Where:** `~/.nandi/repo/.nandi/agents/<id>/agent.md`. `src-tauri/src/services/watcher.rs`, `src-tauri/src/services/repo_store.rs`, format in `src-tauri/src/services/domain/agent.rs`.
- **Preconditions:** App running.
- **Steps:**
  1. Create `~/.nandi/repo/.nandi/agents/shell-helper/agent.md` with the example from userguide §4.
  2. Edit its `description:`.
  3. Break the frontmatter (remove the closing `---`).
- **Expected:** Step 1: the agent appears on the Agents page without restart. Step 2: the description updates. Step 3: the agent keeps its previous definition; an error is reported (row may show broken state if it never loaded).
- **Edge / negative cases:**
  - Folder names that aren't valid IDs (uppercase, spaces) are not agents.
  - Editor temp files (`.swp`, `~`, `.tmp`) are ignored.
  - A BOM and CRLF line endings parse correctly.
- **Automation:** Covered by: Rust tests in `src-tauri/src/services/repo_store.rs` (`loads_reloads_and_keeps_the_last_good_version`), `src-tauri/src/services/domain/agent.rs` tests | Layer: api | Gap: watcher latency is manual.
- **Needs relay:** no
- **Status:** Shipped

### AG-08 — agent.md format: keys the app honors

- **What:** The frontmatter keys parsed or read today, with their effect:
  - `name`, `description`; `model` (`<provider>:<model>`, prefixes `claude:`, `gpt:`, `deepseek:`, `ollama:`; omitted = settings default model).
  - `image`, `memory_mb` (alias `memory-mb`), `cpu`: container image and resources. `shell_timeout_s` (1–3600, default 30): how long one `shell` call may run (RT-06).
  - `allowed-tools` (alias `allowed_tools`; omitted = all runtime tools), `requires_approval` (alias `requires-approval`).
  - `pilots`, `aws` (`services`, `region`), `network` (`allow` hosts): see the pilots/egress sections.
  - `serves`, `provides`, `accepts_delegation`, `max_delegation_concurrent`, `clarification` (policy: `allow`, `max_rounds`, `auto_topics`, `escalate_topics`, `timeout_seconds`).
  - `runtime` (`native`, `claude-code`, `codex`, `gemini-cli`, `custom:<id>`) and `runtime_config` (`model`, `provider: bedrock`, `gateway: litellm`, `aws_profile`, `aws_region`, `fallback: native`, `max_turns`). Agents always run in a container; there is no placement key. `runtime: nandi` is invalid ("'nandi' is unknown: use native, claude-code, codex, gemini-cli, or custom:<id>").
  - `connectors` (with `folders` for the filesystem connector).
  - `tags` (list of labels, e.g. `[finance]`): policies aim at them with `applies_to: [tag:<label>]` (PO-04); trimmed, empty ones dropped.
  - Turn policy keys from unknown frontmatter: `max_turns` (default 50), `max_cost_usd` (default 5.0), `middlewares` (default `[verify_on_stop, compaction, plan_first, finish_check]`; `[]` disables them; `read_only` adds plan mode), `compaction` (`trigger_tokens`, `preserve_recent_percent` 30, `iterative_merge` true), `memory_disabled` (default false).
  - Body = system prompt; `instructions.md` is appended. Unknown keys are kept on save.
- **Where:** `src-tauri/src/services/domain/agent.rs` (`Frontmatter`, `AGENT_FRONTMATTER_KEYS`, `RuntimeConfig`), `src-tauri/src/services/orchestrator/middleware.rs` (`TurnPolicy::from_extra`), `src-tauri/src/services/agent_memory.rs` (`MEMORY_DISABLED_KEY`), `src-tauri/src/services/a2a/clarification.rs` (`POLICY_KEY`).
- **Preconditions:** None.
- **Steps:**
  1. Write an agent.md using every key above.
  2. Open the agent's Settings tab and Permissions panel; save a change from Settings.
  3. Re-open the file.
- **Expected:** Settings shows name, description, model, image, memory, CPU; Permissions shows the tools, approvals, network hosts and pilots, and the policies aimed at its tags (PO-11). After a Settings save, the unknown and turn-policy keys are still in the file.
- **Edge / negative cases:**
  - `runtime: foo` fails: "'foo' is unknown: use native, claude-code, codex, gemini-cli, or custom:<id>".
  - `runtime_config.gateway: other` or `fallback: other` fail validation.
  - An invalid `max_turns`/`max_cost_usd`/`middlewares`/`compaction`/`memory_disabled` logs a warning and uses the default (the agent still loads).
  - `model:` without a prefix makes the agent error (see AG-03).
- **Automation:** Covered by: Rust tests in `domain/agent.rs` (incl. `every_frontmatter_key_is_documented`), `orchestrator/middleware.rs` (`from_extra_parses_every_key`, `invalid_values_fall_back_to_the_defaults`), `services/agents.rs`, `services/policies.rs` (`tags_in_agent_md_select_policies`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### AG-41 — Agents upgrade to `agent.md` (from Nandi 0.2)

- **What:** An agent's one required file is `agent.md`. A gold copy from Nandi 0.2 (or 0.1.0) has `skill.md`: on the first start of this build, every agent folder's `skill.md` becomes `agent.md` (the person's agents, installed agents and packs), byte for byte, in one commit "Upgrade agents to agent.md". Installed copies still exactly as their signed release (but for the rename) stay unmodified and verified, never re-signed; Nandi's unedited files still take this build's updates and edited ones are offered them. A pack release that still names the file `skill.md` won't install or update.
- **Where:** `~/.nandi/repo/.nandi/agents/`. `src-tauri/src/services/repos/from_0_2.rs` (run first in `AppState::start`), `agent_locator::folders`; `installed.yaml` and `shipped.yaml` re-recorded; state.db migration `011_agent_md.sql` (retro proposals). See `docs/developer/data-model.md#upgrade-from-nandi-02-agentmd`.
- **Preconditions:** A `~/.nandi` from Nandi 0.2.x with an own agent, Nandi edited or not, and a starter pack installed.
- **Steps:**
  1. Quit 0.2, start this build.
  2. `git -C ~/.nandi/repo log -1 --stat`.
  3. Library › Installed: open the pack. Agents page: open each agent.
  4. Restart.
- **Expected:** 2: "Upgrade agents to agent.md" renames every `skill.md` (playbooks in `skills/` keep their names) and lists the installed copies "Still as installed from their signed releases". 3: every agent loads with its edits; the pack shows Verified and no "modified" warning; Check for updates works. 4: no new commit.
- **Edge / negative cases:**
  - A pack folder edited by hand before the upgrade still shows as modified.
  - A folder with both files keeps `agent.md`; the `skill.md` text is in the commit before.
  - A start cut short (the app killed mid-upgrade) finishes on the next start.
  - Installing a pack release with `skill.md`: "The agent '<id>' has no agent.md. Packs made for Nandi 0.2 name it skill.md: ask the publisher for a release made for Nandi 0.3 or later."
- **Automation:** Covered by: Rust `repos/from_0_2_tests.rs` (8 tests: every folder in one commit, signed copies re-recorded and changed ones not, Nandi edited and not, a start cut short, both files, a new install, 0.1.0 through both steps, the hash mapping), `db/migrations_tests.rs` (`retro_proposals_name_agent_md_after_011`) | Layer: api | Gap: a real 0.2 home by hand.
- **Needs relay:** no
- **Status:** Shipped (0.3.0)

### AG-42 — Each agent loads only when it is needed

- **What:** Other agents (Nandi, the ask box, delegation) see an agent's `name` and `description` (and the capabilities it serves), never its `agent.md` body, its rules or its playbooks. The body and rules load when that agent runs; its playbooks are an index in its own prompt and load one at a time when relevant (`skill_get`).
- **Where:** `services/orchestrator/llm_step.rs` (an agent's own prompt), `services/ask/context.rs` (`AgentBrief`), `services/a2a` (provider cards), `services/playbooks.rs`.
- **Preconditions:** Nandi and one more agent with a long body and a playbook.
- **Steps:** Ask Nandi "who can draft a post?" with the run's prompt visible (Settings › Logs, LLM log).
- **Expected:** Nandi's request names the agent with its description; the agent's body appears only in that agent's own runs.
- **Edge / negative cases:** An agent with no description: refused at creation and publishing (AG-05); an older one still loads, and others see only its name.
- **Automation:** Covered by: Rust `services/loaded_when_needed_tests.rs` (`nandis_prompt_names_other_agents_and_never_carries_their_bodies`, `an_agents_body_loads_when_it_runs_and_its_playbooks_on_demand`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### AG-09 — Agent status

- **What:** An agent's status is Running, Idle, Waiting for approval or Error. It comes from its active run (running / awaiting approval), a running Docker container labelled `nandi.agent.id=<id>`, or an orchestrator that crashed (error, with the error as the description). The UI applies the live current run on top (`effectiveStatus`).
- **Where:** Status pill/dot on the list, page header and sidebar. `src-tauri/src/services/agents.rs` (`apply_run_status`, `scan_agents`), `src/components/agent-console/agent-status.ts`, `AgentStatusIndicator.tsx`.
- **Preconditions:** Docker for the container case.
- **Steps:**
  1. Start an agent's container (AG-19) without sending a message.
  2. Send a message that needs approval.
  3. Approve and let it finish.
- **Expected:** Running while the container is up; Waiting for approval while the approval is pending; Idle after the run ends and the container is stopped.
- **Edge / negative cases:** Docker unavailable: every agent is Idle (warning logged).
- **Automation:** Covered by: Rust tests in `services/agents.rs` | Layer: api | Gap: no UI test for `effectiveStatus`.
- **Needs relay:** no
- **Status:** Shipped

## Agent page

### AG-10 — Agent page shell and header

- **What:** One agent's page with tabs Overview, Chat, Runs, Settings. Header: back arrow ("Back to Agents"), avatar, name, status pill, subtitle (description · runtime · model, or "In a container" for a CLI runtime), **Edit in Ask** (local agents only) and a "⋯" menu: Chat, Publish (local only), "Sharing settings…", Copy agent ID (toast "Agent ID copied"), the container status ("Container: stopped", "Container: running", …) with Start container / Stop container / Restart container, "Delete agent…" (local only). A deep link to a message opens Chat and highlights it; links can also name a tab.
- **Where:** Sidebar → Agents → <agent>. `src/components/views/AgentView.tsx`, tabs in `agent-page-model.ts` (`AGENT_TABS`).
- **Preconditions:** An agent.
- **Steps:**
  1. Open an agent; check the default tab.
  2. Use "⋯" → Copy agent ID.
  3. Navigate to an unknown agent ID (e.g. an old link).
- **Expected:** Opens on Overview. Clipboard holds the folder ID and a toast confirms. An unknown ID shows "This agent no longer exists" ("It may have been deleted or renamed.") with **Back to Agents**. An agent created after app start loads its snapshot on open (skeleton first).
- **Edge / negative cases:**
  - Installed agents: no Edit in Ask, Publish or Delete.
  - Publish and Sharing dialogs are covered in the marketplace and sharing sections.
  - The status pill follows the agent's live turn: it turns Idle as soon as the run ends, also when the page loaded mid-run (e.g. its tool container started) (`agent-status.test.ts`).
- **Automation:** Covered by: `src/components/layout/AppShell.test.tsx` (navigation) | Layer: ui-e2e | Gap: no direct test of `AgentView.tsx`.
- **Needs relay:** no
- **Status:** Shipped

### AG-11 — Pending-approval banner on the agent page

- **What:** While the agent has pending approvals, the first is shown above the tabs as an approval card; "+N more waiting" switches to Overview, which lists all.
- **Where:** `AgentView.tsx` (`usePendingApprovals`), `ApprovalRequestCard.tsx`.
- **Preconditions:** Docker + image, model key, an agent with `requires_approval: [shell]`.
- **Steps:**
  1. Ask the agent to run a shell command.
  2. Switch between tabs.
- **Expected:** The banner stays across tabs and disappears once the approval is decided (anywhere).
- **Edge / negative cases:** Two approvals waiting (two agents can't share one; use a run that plans two gated calls): "+1 more waiting" link appears.
- **Automation:** Covered by: none | Layer: ui-e2e | Gap: no test.
- **Needs relay:** no
- **Status:** Shipped

### AG-12 — Overview tab: health card

- **What:** "Health (last 7 days)": runs, % completed, average duration (finished runs), money spent, seven day cells, runtime and where it runs, "Container: couldn't start" if the container errored, plus fix buttons: **Pick a model in Settings** (bad model → Settings tab), **Connect a model** (no model → Settings › Models & runtimes), **Add a key** for a CLI runtime that can't sign in.
- **Where:** Agent page → Overview. `src/components/agents/AgentOverviewTab.tsx` (`HealthCard`), `AgentHealthCells.tsx`, data from `list_agent_overviews`.
- **Preconditions:** Runs over several days (or seeded `runs` rows).
- **Steps:**
  1. Open Overview for an agent with completed and failed runs.
  2. Hover the day cells.
- **Expected:** Cells oldest first, last is today: green all completed, amber some failed, red all failed, outlined none. Tooltip "N completed, M failed". Cost line only when cost > 0.
- **Edge / negative cases:** Runs older than 7 days are not counted; no runs → no average and no percent.
- **Automation:** Covered by: `agent-page-model.test.ts` (`dayTone`), Rust tests in `agent_overview.rs` (`runs_land_in_their_day_and_old_ones_are_left_out`, `no_runs_means_no_average`) | Layer: unit + api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### AG-13 — Overview tab: waiting, recent work, queue, about

- **What:** "Waiting for you (N)" with every pending approval card (or "Nothing is waiting for you."); "Recent work" with the last 5 runs and **See all runs** (→ Runs tab); the turn queue strip; "About" with the description, "Used by workflows:" links to workflows that use the agent, and **Chat with this agent**.
- **Where:** `AgentOverviewTab.tsx`.
- **Preconditions:** An agent used by a workflow, with some runs.
- **Steps:**
  1. Open Overview; click a workflow link; go back; click a recent run.
- **Expected:** Workflow link opens the workflow; a run row opens Run Detail. With no runs: "Nothing has run yet." and a **Chat with this agent** button.
- **Edge / negative cases:** Agent file in error: About shows "Its file can't be loaded right now."
- **Automation:** Covered by: none | Layer: ui-e2e | Gap: no test.
- **Needs relay:** no
- **Status:** Shipped

### AG-14 — Runs tab

- **What:** Every run of the agent, newest first, 25 per page with "Load more (n of m)". Each row: relative start time, the input's first line (or "(no message)"), "Part of" (Chat, a workflow, a Request, or the trigger label) and status with duration. Filters: "Search runs…" and status ("Status: any", Running, Awaiting approval, Completed, Failed, Cancelled).
- **Where:** Agent page → Runs. `src/components/agents/AgentRunsTab.tsx` (`AgentRunRow`, `RunOriginLink`), `runOrigin` in `agent-page-model.ts`; command `list_all_runs` with `agentId` (`src-tauri/src/rpc/runs.rs`, `services/runs.rs`); HTTP `POST /api/rpc/list_all_runs` with `filters.agentId`.
- **Preconditions:** An agent with runs from chat, a workflow and a trigger.
- **Steps:**
  1. Filter by Failed; type a word from one run's input.
  2. Click a workflow "Part of" link; then a Request link.
  3. Click a row.
- **Expected:** Filters narrow the list; "No runs match." offers **Clear filters**. Workflow link opens the workflow run (work item); Request link opens the Request (`request_for_run`), or toast "That request is no longer available." if none. Row click (or Enter) opens Run Detail.
- **Edge / negative cases:**
  - Trigger origins are plain text, not links.
  - Search also matches the agent, workflow and trigger names (server side).
  - Compacted runs show their status but the filter has no "Compacted" option.
- **Automation:** Covered by: `agent-page-model.test.ts` (`runOrigin`), `src/hooks/useRuns.test.ts`, Rust tests in `services/runs.rs` | Layer: api + ui-e2e | Gap: tab itself untested.
- **Needs relay:** no
- **Status:** Shipped

### AG-15 — Settings tab: General, save, revert and leave guard

- **What:** Plain-words form for name and description. Changes show a sticky bar with **Revert** and **Save**; Save writes agent.md through the repo (`repo_save_agent`) and toasts "Settings saved". Leaving the Settings tab with unsaved changes asks "Discard changes?" ("Your unsaved settings will be lost.", with Keep editing and Discard). The form reloads whenever the files change (e.g. Ask saved). Installed agents are read-only with "Installed from the Library. Make a copy in Ask to change it."
- **Where:** Agent page → Settings. `src/components/agents/AgentSettingsTab.tsx`, `src/hooks/useAgentEditor.ts` (`useAgentDefinition` via `loadEntity`, `useSaveAgent`, `validateDraft`, `toDefinitionInput`), `src-tauri/src/rpc/repo.rs` (`repo_save_agent`).
- **Preconditions:** A local agent.
- **Steps:**
  1. Change the name; switch to Chat; choose Keep editing; then Save.
  2. Clear the name.
  3. Open Settings on an installed agent.
- **Expected:** Leave prompt appears; after Save the header and list show the new name and agent.md has it (staged in git). Empty name shows a field error and Save is disabled. Installed agent: fields disabled, lock notice.
- **Edge / negative cases:**
  - A model format error on save shows "Pick a model above, then save." instead of the raw error.
  - agent.md that can't be loaded: Settings shows an error alert (badModel text when the model is the issue) with **Edit in Ask**.
- **Automation:** Covered by: `src/hooks/useAgentEditor.test.ts` | Layer: ui-e2e | Gap: no component test for `AgentSettingsTab.tsx`.
- **Needs relay:** no
- **Status:** Shipped

### AG-16 — Settings tab: Model and runtime

- **What:** Runtime radio (Nandi plus each installed CLI runtime; not-installed ones are greyed). For Nandi: Provider select (Default (<model>), Anthropic, OpenAI, DeepSeek, Ollama; providers without a key are disabled and marked "add a key") and a Model select by display name; a provider with no key shows a note with **Add a key**. Uninstalled CLI runtimes say "Not installed · set up in Settings › Models & runtimes". For a CLI runtime: "Where it runs" with the fixed text "In an isolated container. Agents never run directly on this computer." and a free-text CLI model field (`runtime_config.model`), with a sign-in note when the runtime's container can't authenticate.
- **Where:** Agent page → Settings → Model. `ModelSection` in `AgentSettingsTab.tsx`; `providerChoices`, `modelOptions` in `agent-page-model.ts`.
- **Preconditions:** At least one provider key; optionally Ollama and a CLI runtime image.
- **Steps:**
  1. Pick DeepSeek and a model; Save.
  2. Pick the default option; Save.
  3. Switch the runtime to Claude Code; set the CLI model to `sonnet`; Save.
- **Expected:** agent.md gets `model: deepseek:<id>`, then no `model:` key, then `runtime: claude-code` with `runtime_config.model: sonnet`. A saved model not in the catalog is kept in the list (opening the tab never changes it).
- **Edge / negative cases:**
  - Ollama not running: Ollama option disabled.
  - A CLI runtime has no "Where it runs" choice: it always runs in a container.
- **Automation:** Covered by: `agent-page-model.test.ts`, `src/lib/models.choices.test.ts` | Layer: unit + ui-e2e | Gap: form untested.
- **Needs relay:** no
- **Status:** Shipped

### AG-27 — Settings tab: Your rules (kept across updates)

- **What:** The agent's `instructions.md`, its user's standing rules, editable on the Settings tab for every agent, installed ones included.
  - For your own agents it writes `instructions.md` in the agent's folder.
  - For an installed (read-only) agent it writes an override, `.nandi/overrides/<id>/instructions.md`. The agent loads the override in place of the shipped file. The installed copy isn't touched (no _Modified_), so updates still apply and keep your rules.
  - **Reset to default** (shown only while an override exists) deletes it.
  - The hint under the title says which case applies.
  - Ask can do the same: `nandi_save_artifact` kind `instructions`.
- **Where:**
  - `src/components/agents/AgentSettingsTab.tsx` (`YourRules`), `src/hooks/useAgentEditor.ts` (`useSaveAgentInstructions`, `useResetAgentInstructions`).
  - Commands `repo_save_agent_instructions`, `repo_reset_agent_instructions`. HTTP `POST /api/rpc/repo_save_agent_instructions|repo_reset_agent_instructions`.
  - `services/repos/agent_repo.rs` (`save_instructions`, `reset_instructions`, `has_instructions_override`, `instructions_source`), `repos/layout.rs` (`instructions_override_path`), `repos/mod.rs` (the fingerprint includes the override), `watcher.rs` (an override file reloads its agent), `studio/save.rs` (`save_instructions`).
- **Preconditions:** An installed pack agent (e.g. PR Reviewer from the Nandi Dev Pack).
- **Steps:**
  1. Agent page › Settings › Your rules: add "Always answer in French." and click Save rules.
  2. Ask the agent something.
  3. Library › Installed: the pack isn't Modified. Run Check for an update.
  4. Click Reset to default and ask again.
- **Expected:**
  - The override file appears and the answer is in French.
  - The pack shows no Modified badge, and an update keeps the rules.
  - After Reset, the shipped rules are back and the answers are in English.
- **Edge / negative cases:** saving empty rules on an installed agent keeps an empty override (no rules) until Reset; for a local agent, empty removes `instructions.md`; an unknown agent is an error.
- **Automation:** Covered by: `agent_repo.rs` (`an_installed_agents_rules_are_the_users_and_survive_an_update`, `a_local_agents_rules_go_in_its_own_folder`), `watcher.rs` (`classifies_gold_paths`), `studio/save.rs` (kinds) | Layer: unit + manual | Gap: no component test for `YourRules`.
- **Needs relay:** no
- **Status:** Shipped

### AG-28 — My notes: the notes Nandi keeps

- **What:** A **My notes** page (left menu, after Ask) for the user's Markdown notes: role, what they own, deadlines, who to ask, runbooks. Nandi reads them to answer, and writes them as it learns things.
  - **The folder** is the one already shared as `my-notes` (Library › Tools › Filesystem), else `~/.nandi/notes`, created empty and shared read-and-write at the first launch (in the background), so Nandi can keep notes before anyone opens My notes. The header shows its path.
  - **The list** shows each `.md` note by its first heading (else its file name), top-level first, then each folder (`runbooks/`). Hidden files and non-Markdown files are left out.
  - **A note** opens in **Read** (rendered Markdown) or **Edit** (a CodeMirror Markdown editor; Ctrl/Cmd+S saves). **Save** shows only with changes. Switching notes or leaving the page with unsaved changes asks first. **Delete** asks first.
  - **New note** takes a name (`deadlines`, `runbooks/etl`), adds `.md` and starts it with a heading.
  - **"Let Nandi keep them up to date"** makes the share read-and-write. Then `filesystem__write_file` into `my-notes/…` doesn't ask for approval (`notes::is_notes_write`), even when the agent's agent.md lists `write_file` under `requires_approval` (the registry requires that); writes anywhere else still ask. Turning it off makes the share read-only, and the connector refuses writes.
  - **Empty:** "No notes yet", with **Open Nandi**.
  - Nandi's own instructions (its `agent.md`, shipped with Nandi) decide what to note and how to arrange it, and to raise what's coming up. The app doesn't impose a structure.
- **Where:**
  - `src/components/notes/` (`MyNotesView.tsx`, `MarkdownEditor.tsx`, `notes-model.ts`), `src/hooks/useNotes.ts`, navigation kind `notes`.
  - `services/notes.rs` (`NotesFolder`, `check_name`, `folder`, `my_notes`, `set_agents_can_write`, `is_notes_write`), `services/connectors.rs` (`shared_folder`, `share_folder`), `orchestrator.rs` (approval exemption).
  - Commands `get_my_notes`, `read_note`, `save_note`, `delete_note`, `set_notes_agents_can_write`. HTTP `POST /api/rpc/{command}`. The folder is watched: every change arrives as `live://changed` (DT-43).
- **Preconditions:** None: Nandi, the default agent, comes with the app (AG-30).
- **Steps:**
  1. Open My notes.
  2. Open a note, switch to Edit, change a line, Ctrl+S.
  3. Turn on "Let Nandi keep them up to date".
  4. Tell Nandi "the SOC 1 walkthrough moved to Nov 19, and Lena is now backup for billing".
- **Expected:**
  - Step 1: the folder path and the notes, the first one open.
  - Step 2: "Saved {name}", and the file on disk has the change.
  - Step 4: no approval card. It writes `deadlines.md` and `owners.md` (with "Last updated" bumped), says "Noted…", and the open note shows the change within about a second, with "Updated just now by Nandi" (DT-44).
- **Edge / negative cases:**
  - A name like `../x`, `.hidden`, `a/b/c/d` or `what?` is refused.
  - Notes over 256 KB are refused.
  - With the switch off, Nandi's write fails, and it tells the user how to allow it.
  - Writes outside `my-notes/` still ask.
- **Automation:** Covered by: `notes.rs` (5 tests, including `writes_inside_the_notes_folder_dont_ask` and `names_stay_inside_the_folder`), `notes-model.test.ts`, `MyNotesView.test.tsx`, `nav-tree.test.ts`, `Sidebar.test.tsx` | Layer: unit + ui | Live: 2026-10-25 on instance A, as in step 4 | Gap: no test of the leave guard on this page.
- **Needs relay:** no
- **Status:** Shipped

### AG-30 — Nandi, the default agent, comes with the app

- **What:** Nandi, the default agent (ID `nandi`), is built into the app, not installed from a pack.
  - On every start (and when a repo is fetched from a remote), the app writes `.nandi/agents/nandi/agent.md` and `instructions.md` if `agent.md` is missing, and stages them. A new repo's first commit includes it.
  - It's an ordinary local agent: no install, consent or fork. The user edits its skill and **Your rules** like any agent of their own; Backup commits and pushes its history with the rest of the repo. An existing folder is never overwritten.
  - It can't be deleted: Delete is hidden in its menu, page and Settings, and `delete_agent` refuses it; **Reset to defaults** takes its place (AG-36). If its `agent.md` is removed by hand, the next start puts the shipped one back and keeps the user's `instructions.md`.
  - It is not one more agent. It has its own **Nandi** item at the top of the sidebar (AG-36); the list under Agents, the All agents count and the Agents page hold only the person's own and installed agents.
  - Pickers show it as a fixed **Nandi** choice above the person's agents: a trigger's target, a workflow step's agent, a template's steps, a pilot's grant, Studio's Agent tab, Settings › Internet access, and Network's "takes requests by name".
  - Home's Get started, the welcome strip and the companion's first-run text mean "none of the person's own agents", so they still suggest a starter pack and never name Nandi as one of the user's agents.
- **Where:** `src-tauri/src/services/builtin_agents.rs` (`DEFAULT_AGENT`, `seed`), `src-tauri/resources/builtin-agents/nandi/`, `repo_bootstrap.rs` (`ensure_scaffolding`), `services/agents.rs` (`AgentInfo.kind`, `AgentKind`, `delete_agent`), `lib/agent-identity.ts` (`isDefaultAgent`), `nav-tree.ts` (`agentNodes`, the `nandi` item), `AgentsListView.tsx`, `agents/AgentOptions.tsx` (the pickers), `dashboard-model.ts`, `CompanionWindow.tsx`, `ReachabilityChecklist.tsx` (`ChooseAgents`), `AgentMenu.tsx`, `AgentView.tsx`, `AgentSettingsTab.tsx`.
- **Preconditions:** A new Nandi home (empty `~/.nandi`).
- **Steps:**
  1. Start Nandi and finish setup without picking a starter pack.
  2. Open Nandi at the top of the sidebar, open its menu and Settings.
  3. In Settings › Your rules, add a line and save. Restart Nandi.
  4. Open My notes with no notes.
- **Expected:**
  - Step 1: Nandi is the sidebar's top item, not under Agents and not on the Agents page; a new trigger's **Run** picker shows Nandi above "Agents"; `git log` in `~/.nandi/repo` shows its files in the first commit.
  - Step 2: no Delete anywhere, Reset to defaults instead; no "installed from the marketplace" banner.
  - Step 3: the line is still there after the restart.
  - Step 4: "No notes yet" with **Open Nandi**.
- **Edge / negative cases:**
  - Installing a pack that contains an agent `nandi` is refused ("you already have nandi").
  - Deleting `.nandi/agents/nandi/agent.md` by hand: the next start restores it.
  - An old `.nandi/agents/chief-of-staff/` folder from a dev build stays as an ordinary agent; nothing reads it. Reset `~/.nandi` to drop it.
- **Automation:** Covered by: `builtin_agents.rs` (`seeds_every_builtin_agent_once`, `keeps_the_users_edits`, `restores_a_removed_skill_but_keeps_the_role`, `the_shipped_skills_parse_and_stay_short`, `no_file_outside_this_one_spells_the_default_agent_id`), `agents.rs` (`builtin_agents_cannot_be_deleted`, `lists_the_default_agent_first`), `repo_bootstrap.rs` (`a_new_install_gets_a_fresh_committed_repo`), `agent-identity.test.ts`, `AgentOptions.test.tsx`, `nav-tree.test.ts` ('gives Nandi its own item…'), `AgentsListView.test.tsx`, `TriggerForm.test.tsx`, `StepDetailsPanel.test.tsx`, `NewWorkflowMenu.test.tsx`, `LibraryDrawer.test.tsx` (the step, template and pilot grant pickers), `Studio.test.tsx` ('switches artifact types'), `InternetAccessSection.test.tsx`, `ChooseAgents.test.tsx`, `dashboard-model.test.ts`, `CompanionWindow.test.tsx` ('greets a first run…') | Layer: unit + ui | Gap: no test of the hidden Delete.
- **Needs relay:** no
- **Status:** Shipped

### AG-36 — Nandi's page and Reset to defaults

- **What:** Nandi has its own page, apart from Agents (owner decisions N-2, N-5).
  - The **Nandi** item sits at the top of the sidebar, by Nandi's own name, with its status dot. It opens Nandi's agent page with the same tabs as any agent: Overview, Chat, Runs, Playbooks and Settings (model, Internet access, Your rules, permissions, connectors, sharing, memory). Its page and runs light the Nandi item, never Agents. The page has no Back to Agents.
  - Settings has a **Nandi's settings** button in its header that opens Nandi's Settings tab.
  - Delete is never offered. **Reset to defaults…** is, in the page's ⋯ menu and in Settings › Danger zone.
  - The confirm names what changes: its settings (`agent.md`), Your rules (back to the `instructions.md` template) and its playbooks (edited ones go back, added ones go). It says that memory, runs, chats, connector grants and other triggers stay, and that the old versions stay in the agents repo's git history. Nothing changes until **Reset to defaults** is pressed.
  - It also puts back Nandi's default triggers, the morning digest and the mail check, as they ship, even ones the person removed (AG-37). Other triggers stay.
  - The reset writes through the agent repository and commits like any gold-copy change. Two git commits: "Keep Nandi's files before a reset to defaults" (the person's files as they were, even if never synced) and "Reset Nandi to defaults". Without a git repository the reset is refused and nothing changes.
- **Where:** `src/store/navigation-store.ts` (`NAV_SECTIONS`, `navSection`), `src/components/layout/nav-tree.ts`, `Sidebar.tsx`, `views/AgentView.tsx`, `agents/AgentSettingsTab.tsx`, `agents/ResetNandiDialog.tsx`, `settings/SettingsView.tsx` (`NandiSettingsLink`), `hooks/useAgentEditor.ts` (`useResetDefaultAgent`); command `reset_default_agent` (`rpc/agents.rs` → `services/agents.rs` → `builtin_agents::reset_default`), `repos/agent_repo.rs` (`replace_playbooks`, `definition_files`).
- **Preconditions:** A Nandi home whose Nandi has an edited name, Your rules, an edited playbook and an added one.
- **Steps:**
  1. Click **Nandi** at the top of the sidebar. Open each tab.
  2. Open Settings, then **Nandi's settings**.
  3. Open the ⋯ menu, **Reset to defaults…**, then Cancel.
  4. Open it again and press **Reset to defaults**.
  5. Run `git log -3 --stat` in `~/.nandi/repo`.
- **Expected:**
  - Step 1: the page opens with every tab; the Nandi item is lit, Agents isn't.
  - Step 2: Nandi's Settings tab opens.
  - Step 3: nothing changes.
  - Step 4: toast "Nandi is back to its defaults"; the name, Your rules and playbooks are the shipped ones, the added playbook is gone; memory is kept.
  - Step 5: the reset commit, and before it the commit holding the edited files.
- **Edge / negative cases:**
  - The gold copy isn't a git repository: the reset fails with that reason and the files stay.
  - Unsaved edits on the Settings tab: the reset writes the files; the form reloads from them.
- **Automation:** Covered by: `builtin_agents.rs` (`a_reset_restores_the_shipped_files`, `a_reset_keeps_the_old_files_in_the_history`, `a_reset_needs_the_gold_copys_history`), `ResetNandiDialog.test.tsx` (the confirm), `nav-tree.test.ts`, `Sidebar.test.tsx`, `navigation-store.test.ts` | Layer: unit + ui | Gap: no UI test of the menu item or the Settings link.
- **Needs relay:** no
- **Status:** Shipped

### AG-37 — Nandi's default schedule

- **What:** Nandi ships with three triggers of its own, on by default (owner decision N-4). They are ordinary cron triggers that target Nandi: the person edits, turns off or removes them like any trigger.

  | Trigger                           | When               | What Nandi does                                                                |
  | --------------------------------- | ------------------ | ------------------------------------------------------------------------------ |
  | Morning digest (`morning-digest`) | Every day at 08:00 | A short brief for the day: meetings, work waiting, work in progress, deadlines |
  | Mail check (`mail-check`)         | Weekdays at 16:00  | Looks over the day's mail for asks with no reply and promised replies          |
  | Weekly retro (`weekly-retro`)     | Fridays at 17:00   | Learns from the week's corrections and proposes changes (AG-40)                |
  - They are written once, into a fresh install, in its first commit. An existing install gets a new one with the update that ships it (AG-38). A trigger the person removes stays removed. **Reset to defaults** (AG-36) puts them back as they ship.
  - The mail check runs only while a mailbox is connected. Without one, each fire is recorded as skipped ("no mailbox is connected") and starts no run. A test fire always runs.
  - The email check that hands new mail to Nandi every few minutes is separate (Settings › Email).

- **Where:** `src-tauri/resources/builtin-agents/nandi/triggers/`, `builtin_agents.rs` (`DEFAULT_TRIGGERS`, `seed_triggers`, `reset_default`), `repo_bootstrap.rs` (`bootstrap`), `triggers/mod.rs` (`FireGuard`), `triggers/dispatch.rs` (`process`), `email/mod.rs` (`MailCheckGuard`), `ResetNandiDialog.tsx`.
- **Preconditions:** A new Nandi home (empty `~/.nandi`).
- **Steps:**
  1. Start Nandi and open Triggers.
  2. Test-fire **Morning digest**.
  3. Delete **Mail check** and restart Nandi.
  4. Reset Nandi to defaults (AG-36) and open Triggers.
- **Expected:**
  - Step 1: Morning digest and Mail check are listed, on, with Nandi as the target.
  - Step 2: a Nandi run starts with the digest prompt.
  - Step 3: Mail check stays gone.
  - Step 4: both are back with their shipped schedules.
- **Edge / negative cases:**
  - No mailbox connected at 16:00 on a weekday: the Mail check fire is skipped, with no run.
  - A home restored from a remote (not fresh) gets no default triggers; its own triggers come with it.
- **Automation:** Covered by: `builtin_agents.rs` (`the_default_triggers_are_nandis_and_on`, `seeds_the_default_triggers_without_overwriting_them`, `a_reset_restores_the_default_triggers_and_keeps_the_others`), `repo_bootstrap.rs` (`a_new_install_gets_a_fresh_committed_repo`, `a_removed_default_trigger_stays_removed`), `triggers/tests.rs` (`a_guard_skips_the_fires_it_holds_but_not_a_test_fire`), `email/tests.rs` (`the_mail_check_trigger_waits_for_a_mailbox`), `ResetNandiDialog.test.tsx` | Layer: unit + ui | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### AG-38 — Updates to Nandi's shipped files

- **What:** A new version of Nandi brings new versions of the files it ships for Nandi: `agent.md`, `instructions.md`, the playbooks and the default triggers. The person's edits are never overwritten.
  - At start, each shipped file is compared with what Nandi last wrote there (`.nandi/shipped.yaml`, by SHA-256):

    | The file                         | What happens                              |
    | -------------------------------- | ----------------------------------------- |
    | Unedited, and the version has it | Replaced by the new version and committed |
    | Edited by the person             | Kept; Nandi's page offers the update      |
    | Deleted by the person            | Stays deleted                             |
    | New in this version              | Added and committed                       |
    | From Nandi 0.1.0 (no record yet) | Edited or not by the hashes 0.1.0 shipped |

  - The commit is "Update Nandi's shipped files", naming the files updated and added.
  - For an edited file, Nandi's page shows **An update to Nandi's instructions is available** with each file, a diff from the person's version to the new one, and **Take update** / **Keep mine**.
  - **Take update** commits the person's version first ("Keep my … before taking Nandi's update"), then the new one ("Take Nandi's update to …").
  - **Keep mine** records the choice in `.nandi/shipped.yaml` and commits it. That version isn't offered again; a later one is.
  - **Reset to defaults** (AG-36) records the files it writes and forgets every Keep mine.
- **Where:** `services/shipped_files.rs` (`update`, `decide`, `offers`, `take`, `keep_mine`, `restore`), `repo_bootstrap.rs` (`bootstrap`, `commit_shipped_update`), `builtin_agents.rs` (`reset_default`); commands `default_agent_updates`, `take_default_agent_update`, `keep_default_agent_file` (`rpc/agents.rs` → `services/agents.rs`); `agents/NandiUpdateCard.tsx`, `agents/nandi-update-model.ts`, `hooks/useAgentEditor.ts`, `views/AgentView.tsx`.
- **Preconditions:** A Nandi home from an older build where Nandi's Your rules was edited and `agent.md` wasn't, and a newer build whose shipped `agent.md` and `instructions.md` differ.
- **Steps:**
  1. Start the newer build. Run `git log -1 --stat` in `~/.nandi/repo`.
  2. Open Nandi's page.
  3. Press **Keep mine**, then restart Nandi and open its page.
  4. Edit Your rules again on a build with a newer `instructions.md`, open Nandi's page and press **Take update**.
- **Expected:**
  - Step 1: the commit "Update Nandi's shipped files" changes `agent.md` only.
  - Step 2: the card names `instructions.md` with the diff; nothing else on the page changed.
  - Step 3: the card is gone and stays gone.
  - Step 4: Your rules is the shipped version; `git log -2` shows the person's version, then the update.
- **Edge / negative cases:**
  - A playbook the person deleted stays deleted, even when the new version changed it.
  - A new playbook or default trigger is added even when its folder exists.
  - A malformed `.nandi/shipped.yaml`: nothing is updated; the error is logged.
- **Automation:** Covered by: `shipped_files_tests.rs` (`decides_by_what_nandi_wrote_what_is_there_and_what_ships`, `an_update_replaces_unedited_files_keeps_edited_ones_and_adds_new_ones`, `a_gold_copy_from_0_1_0_tells_edited_from_unedited_by_its_hashes`, `take_update_writes_this_builds_version_and_commits_it`, `keep_mine_is_remembered_until_a_newer_version_ships`, `a_reset_writes_the_shipped_files_records_them_and_forgets_keep_mine`), `NandiUpdateCard.test.tsx` | Layer: unit + ui | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### AG-31 — Nandi browses websites

- **What:** Nandi browses websites itself, in a browser the user watches. There is no separate Browser agent.
  - Its `agent.md` declares the visible **Browser** pilot (never headless) with `allow: [interact]`: clicking and typing run, submitting, secrets, uploads and paying wait for approval. Sign-ins, CAPTCHAs, payments and passwords go to the user with the pilot's hand-over.
  - It ships with no allowed sites. A new site shows the "allow this site?" card with its reason, and it waits.
  - Only the user's chats and intake threads get the browser. The email and calendar check runs never do.
  - Its behaviour is in its `agent.md` ("Website work", body 900 words or fewer).
- **Where:** `resources/builtin-agents/nandi/agent.md` ("Website work", `pilots`), `services/orchestrator/llm_step.rs` (`data_run_allows`, `agent_pilot_tools`).
- **Preconditions:** A new Nandi home. The Browser pilot installed and started in Library › Tools, with Chrome or Edge on the computer.
- **Steps:**
  1. Ask Nandi: "Check what example.com says on its home page."
  2. Allow the site when the card asks, and allow the pilot when asked.
  3. Ask it to fill in a sign-in form on a site.
- **Expected:**
  - Step 1–2: a Chrome or Edge window opens on example.com; Nandi reports what it found with the link.
  - Step 3: it hands over to you and never types a password.
- **Edge / negative cases:**
  - A form submit waits for approval.
  - Don't allow: it says what it needed the site for and what is left undone.
  - A mail or calendar check run has no `pilot_browser__*` tools.
  - A dev home that seeded the old Browser agent still has its folder; delete `.nandi/agents/browser` by hand.
- **Automation:** Covered by: `builtin_agents.rs` (`nandi_browses_with_the_visible_browser`, `the_shipped_skills_parse_and_stay_short`), `orchestrator.rs` (`a_check_started_run_never_gets_pilot_tools`) | Layer: unit | Gap: no automated run with the real browser pilot.
- **Needs relay:** no
- **Status:** Shipped

### AG-32 — Run plans: plan first, live checklist, finish check

- **What:** Every agent plans a run that changes something, step by step, and finishes it.
  - Two host tools for every agent (not in email or calendar check runs, and not when the agent has no tools): `plan_set` (steps with title, kind `navigate | read | fill | submit | handover | approval | delegate | verify | cleanup | other`, `evidence: screenshot | none`, note) and `plan_update` (one step's status `pending | doing | done | skipped | failed`, result, evidence files). One step is `doing` at a time. A later `plan_set` replaces the plan and keeps the old one as a revision. Plans are run data (`run_plans`), never files.
  - **Plan first:** the first call that changes something (browser clicking, typing, submitting; `work_start`, `work_update`, `delegate_to_agent`; writing files outside the notes) is refused with "make a plan first (`plan_set`)" while the run has no plan. Reads, notes and the plan tools never are. Shell commands aren't classified.
  - **Risky steps are the user's:** a browser submit, payment, secret or hand-over runs only while the `doing` step is `submit`, `handover` or `approval`; `work_start` and `delegate_to_agent` only while it is `delegate` or `approval`. The refusal says which kind to plan. Existing approvals still apply on top.
  - **Finish check:** once per run, a run that ends with open steps, a `screenshot` step without evidence, or its browser still open gets one continuation asking it to finish, close the browser (unless the person is still using a page in the visible browser: a video, music, something to read) and report each step.
  - **Safety net:** when a run ends, Nandi closes that agent's headless browser pages (the profile stays). The visible browser stays open for the person, and the sites its runs were allowed once stay allowed until that window closes.
  - **Screenshots:** only on steps that change something for good (a form sent, a payment, a booking), never on opening, reading or playing.
  - **Screenshots:** saved as `browser/<run>/<step>-<n>.png` in the agent's files and linked to the `doing` step; refused during a `handover` step; removed after 30 days; never in an S3 backup.
  - **Live checklist:** a card with each step, its status, its result and screenshot thumbnails (click opens the file on the desktop), in the chat, the intake thread and Run Detail › Overview, updated by `run://plan_updated`. `plan_update` calls are hidden; a refused plan call shows as a tool card.
  - agent.md `middlewares:` without `plan_first` / `finish_check` turns them off (they are on by default).
- **Where:** `src-tauri/src/services/run_plan/` (`mod.rs` model, `classify.rs` what changes something, `store.rs`, `tool.rs`, `screenshots.rs`), `services/orchestrator/run_plan.rs` (the two middlewares), `services/orchestrator/tool_calls.rs` (`run_call`, `screenshot_call`), `services/orchestrator/call_gate.rs` (`plan_check`), `services/pilots/browser.rs` (`close_pages`, `has_open_pages`), `services/events.rs` (`turn_done`), `src/components/agent-console/RunPlanCard.tsx`, `run-plan-model.ts`, `src/hooks/useRunPlan.ts`.
- **Preconditions:** An agent with tools (Nandi with the Browser pilot started for the browser cases).
- **Steps:**
  1. Ask Nandi to open example.com and take a screenshot of the home page.
  2. Watch the chat while it works.
  3. Open the run in Run Detail.
  4. Ask an agent to write a file without planning (a custom agent whose instructions say "never plan").
- **Expected:**
  - Step 1–2: it sets a plan first; a "Plan" card shows each step, the one being done highlighted, and a thumbnail under the screenshot step. The browser closes at the end, and the last message reports each step.
  - Ask Nandi to play a YouTube video: it opens and starts it, takes no screenshot, and the run ends with the window open and the video still playing ("I left the browser open"). Close the window: the video's sites are no longer allowed.
  - Step 3: the same card is on the Overview tab.
  - Step 4: the write is refused with "make a plan first"; the agent plans and the write runs.
- **Edge / negative cases:**
  - A payment click while a `navigate` step is doing is refused, naming `submit`, `handover` or `approval`.
  - A screenshot during a `handover` step is refused.
  - Ending with a step still pending: one continuation, never two.
  - Email and calendar check runs have no plan tools and are never refused for lack of a plan.
  - A screenshot older than 30 days shows "Screenshot removed".
- **Automation:** Covered by: Rust tests in `run_plan/` (`plan_update_moves_one_step_at_a_time`, `effects_table`, `risky_calls_need_a_covering_doing_step`, `a_handover_step_refuses_screenshots`, `old_screenshots_go_and_recent_ones_stay`, `backups_leave_browser_files_out`, `a_later_plan_set_is_kept_as_a_revision`), `orchestrator/middleware_tests.rs` (`a_change_before_a_plan_is_blocked_and_runs_once_planned`, `a_risky_call_needs_a_doing_step_that_covers_it`, `the_finish_check_fires_once_per_run`, `an_open_browser_gets_the_finish_check_and_is_closed_at_run_end`, `screenshots_are_named_per_step_linked_and_refused_during_a_handover`), `orchestrator.rs` (`a_check_started_run_has_no_plan_tools`), Vitest `run-plan-model.test.ts`, `RunPlanCard.test.tsx` | Layer: unit, integration | Gap: no automated run with the real browser pilot.
- **Needs relay:** no
- **Status:** Shipped

### AG-33 — Playbooks: skill files an agent reads on demand

- **What:** An agent's step-by-step guides, `skills/<name>.md` next to its agent.md.
  - Frontmatter `name` (kebab-case, the file name), `description` and `when` (one line each), then the steps (900 words or fewer). No `model:` or other keys.
  - Every run with tools (not the email or calendar checks) gets a "Your playbooks" index (name, description, when) and two host tools: `skill_list` and `skill_get` (one body by name). Neither needs a run plan. Broken files are left out of runs.
  - **Playbooks tab** on the agent page: the list (name, description, when), the open playbook in a Markdown editor with a word count, **New playbook**, **Save playbook** (checks the same rules: taken name, kebab-case, required fields, word limit), **Delete** (asks first). Leaving with unsaved changes asks first. Broken files show with their error.
  - An installed agent's playbooks are read-only (no New, Save or Delete). Installs bring `skills/*.md`.
  - Nandi ships eight, added once when it has no `skills/` folder: `web-task`, `pay-a-bill`, `sign-in-and-accounts`, `delegate-work`, `build-a-workflow`, `email-triage`, `meeting-prep`, `occasions`. Edits and deletions stay.
- **Where:** `src-tauri/src/services/domain/playbook.rs`, `services/repos/agent_repo.rs` (`load_playbooks`, `save_playbook`, `delete_playbook`), `services/repo_service.rs`, `rpc/repo.rs`, `rpc/repo.rs`, `services/playbooks.rs`, `services/orchestrator/llm_step.rs` (offer), `services/orchestrator/tool_calls.rs` (`run_call`), `services/builtin_agents.rs`, `src-tauri/resources/builtin-agents/nandi/skills/`, `src/components/agents/AgentPlaybooksTab.tsx`, `playbooks-model.ts`, `src/hooks/usePlaybooks.ts`.
- **Preconditions:** None (Nandi is built in).
- **Steps:**
  1. Open Agents › Nandi › Playbooks.
  2. Open `pay-a-bill`, add a line, **Save playbook**.
  3. **New playbook**: name `weekly-report`, when "Friday afternoon", what it covers, a few steps; save.
  4. In Nandi's chat: "Pay my Acme Energy bill".
  5. Delete `weekly-report`.
  6. Open an installed pack agent's Playbooks tab.
- **Expected:**
  - Step 1: eight playbooks, each with its description and when; the first is open.
  - Step 2: "Playbook saved"; `skills/pay-a-bill.md` holds the change.
  - Step 3: it appears in the list; a name like `Weekly Report` becomes `weekly-report`; a taken name or an empty field keeps Save disabled with the reason.
  - Step 4: the agent reads `pay-a-bill` (`skill_get`) before it plans, and the plan follows it (sign-in and payment as hand-overs).
  - Step 5: asks first, then it's gone from the list and the folder.
  - Step 6: read-only notice; no New, Save or Delete.
- **Edge / negative cases:**
  - A file with `model:` or a name that isn't its file name shows as broken, and runs leave it out.
  - A body over 900 words can't be saved.
  - Deleting a built-in playbook and restarting doesn't bring it back.
- **Automation:** Covered by: Rust tests in `domain/playbook.rs` (parse, name, required fields, 900 words, no `model:`, unique names, the index), `repos/agent_repo.rs` (`playbooks_are_written_read_renamed_and_removed`, `a_broken_playbook_is_a_problem_not_a_failure`, `an_installed_agents_playbooks_are_read_only`), `playbooks.rs` (`skill_list`, `skill_get`), `builtin_agents.rs` (`seeds_the_playbooks_once_and_keeps_the_users_changes`, `the_shipped_playbooks_parse_and_stay_short`, `the_shipped_playbooks_name_only_real_tools`), `orchestrator/middleware_tests.rs` (`skill_get_answers_from_the_agents_playbooks`), `run_plan/classify.rs` (`effects_table`), `marketplace/install.rs` (`an_agent_brings_its_playbooks_but_no_scripts`), Vitest `playbooks-model.test.ts`, `AgentPlaybooksTab.test.tsx` | Layer: unit, integration | Gap: no check that a model actually follows a playbook.
- **Needs relay:** no
- **Status:** Shipped

### AG-34 — Sites and accounts: site secrets, site profiles, sign out everywhere

- **What:** What an agent's browser knows about the user's sites, managed on the agent's **Permissions** tab, under each browser pilot.
  - Each secret the browser types, with **Secret saved** or **Not set** (never the value). Type a value and **Save** to set or replace it, **Clear** to remove it.
  - **Add secret** (agents made on this computer, Nandi too): a name (typed in capitals, `ACME_PASSWORD`) and a value. The name goes into agent.md (`secrets: {ACME_PASSWORD: nandi.pilot.<agent>.acme-password}`), the value into the keychain. A taken or badly formed name keeps Add disabled with the reason. Pack agents show no Add; their values can still be set.
  - **Sign out everywhere** asks first, then stops that agent's browser and deletes its profile (cookies, sign-ins). Disabled with "No sign-ins saved" when there is none. Secrets stay.
  - Browser profiles are never backed up (S3 Backup leaves out `local/browser/*/*/profile`, My Nandi browser's too) and live under `~/.nandi/local`, not the repo.
  - **Sign out everywhere** here clears the agent's own profile only. My Nandi browser (AG-39) is signed out in Settings › Browser (ST-37).
  - Site profiles: Nandi keeps non-secret details in `my-notes/private/sites/<site>.md` (its `sign-in-and-accounts` and `web-task` playbooks). A note with a card number (a Luhn-valid 13 to 19 digit run) or a `password:` line is refused with a message saying what to keep instead.
- **Where:** `src/components/agent-console/PermissionsPanel.tsx`, `site-secrets-model.ts`, `src/hooks/usePilots.ts`; `src-tauri/src/services/pilots.rs` (`add_pilot_secret`, `set_pilot_secret`, `clear_browser_profile`), `services/pilots/browser.rs` (`sign_out`, `mask_secrets`, `profile_backup_exclude`), `services/domain/capability.rs` (`pilot_secret_ref`), `services/domain/note_guard.rs`, `services/connectors/filesystem.rs`, `services/connectors/s3_backup.rs`.
- **Preconditions:** The Browser pilot installed; Nandi allowed to use it.
- **Steps:**
  1. Open Agents › Nandi › Permissions. Under Browser, add `ACME_PASSWORD` with a value.
  2. Open the agent's agent.md (Settings or Studio).
  3. Ask Nandi to open a site that remembers a sign-in, then press **Sign out everywhere** and confirm.
  4. Ask Nandi: "Note my Acme card 4242 4242 4242 4242 in the acme.com profile".
  5. Open an installed pack agent with a browser pilot.
- **Expected:**
  - Step 1: "Secret ACME_PASSWORD added"; the row shows **Secret saved**; the form is empty again.
  - Step 2: `secrets: { ACME_PASSWORD: nandi.pilot.nandi.acme-password }` under the browser; no value anywhere.
  - Step 3: "Signed out everywhere"; the next run's browser starts signed out; the button is disabled.
  - Step 4: the write is refused ("looks like a card number"); the agent says it won't keep it.
  - Step 5: secret values can be set; no **Add secret**.
- **Edge / negative cases:**
  - A name starting with a digit, or one already listed, can't be added.
  - A value is never shown again, in the panel, the chat, Run Detail or the logs.
  - `Secret name: ACME_PASSWORD` in a site profile is written.
- **Automation:** Covered by: Rust `pilots::tests::a_site_secret_s_value_stays_in_the_keychain` (agent.md, keychain, Permissions view, events, prompt note, log lines), `domain/agent.rs` (`a_site_secret_added_by_the_user_round_trips_by_name`), `domain/note_guard.rs`, `connectors/filesystem.rs` (`the_notes_keep_site_profiles_but_never_cards_or_passwords`), `pilots/browser.rs` (`signing_out_clears_only_that_profile`, `secret_values_never_reach_a_reply`, `backups_leave_out_every_browser_profile`, `profiles_live_under_local_never_the_repo`), Vitest `site-secrets-model.test.ts`, `PermissionsPanel.test.tsx` | Layer: unit, integration | Gap: no end-to-end run against a real site.
- **Needs relay:** no
- **Status:** Shipped

### AG-39 — An agent's browser uses My Nandi browser (a grant)

- **What:** The visible **Browser** pilot signs in with its own profile (the default) or with **My Nandi browser (Chrome|Edge)**, the shared profile the person signs in to in Settings › Browser (ST-37).
  - The agent's **Permissions** tab shows **Signs in with** under Browser: **Its own profile** or **My Nandi browser (Chrome)** / **(Edge)**, one per installed browser. Choosing one is a grant the person gives this agent: kept on this computer (`consent.<agent>.browser.profile` in `state.db`), recorded in the decisions file (kind `browserProfileGrant`, granted or removed), and listed on the Settings › Browser card. Choosing **Its own profile** takes it back.
  - Everything else stays: allowed sites, per-call approvals, the visible window, the egress proxy as `browser:<agent>`.
  - The headless browser never gets it: Browser (headless) shows no choice and always uses its own profile.
  - One browser per profile: if the person's window or another agent has it open, the agent's browser call answers "My Nandi browser (Chrome) is in use by <agent> right now. Try again when its run ends." (or "is open in another window"). It doesn't wait. The profile is free again when the holder's run ends or its browser closes.
  - A granted browser that was uninstalled stays listed; the agent's browser then says to choose another.
- **Where:** `src/components/agent-console/PermissionsPanel.tsx`, `BrowserProfileSelect.tsx`, `src/lib/browser-profile.ts`, `src/hooks/usePilots.ts` (`useSetAgentBrowserProfile`); `src-tauri/src/services/pilots.rs` (`set_agent_browser_profile`, `agent_pilot_permissions` `sharedProfile`), `services/pilots/store.rs` (`browser_profile`, `set_browser_profile`, `browser_profiles`), `services/pilots/profiles.rs` (`for_pilot`, `claim_for_agent`, `release_agent`), `services/pilots/browser.rs` (`ensure`, `call`, `close_pages`); command `set_agent_browser_profile`.
- **Preconditions:** The Browser pilot installed and allowed for two agents; Chrome installed; signed in to a site in My Nandi browser (Chrome).
- **Steps:**
  1. On agent A's Permissions, set **Signs in with** to **My Nandi browser (Chrome)**.
  2. Ask agent A to open the site you signed in to.
  3. While A's run is open, ask agent B (also granted Chrome) to browse.
  4. Open Settings › Logs › Decisions (or `.nandi/decisions/<yyyy-mm>.md`).
  5. Set A back to **Its own profile** and ask it to open the site again.
- **Expected:**
  - Step 1: the hint says it uses your sign-ins from My Nandi browser; Settings › Browser lists A under Chrome.
  - Step 2: the window opens signed in; the site still has to be in A's allowed sites, and clicks still ask.
  - Step 3: B says My Nandi browser (Chrome) is in use by A right now; nothing else is harmed.
  - Step 4: `browserProfileGrant` granted for A.
  - Step 5: the window opens on A's own profile, signed out of that site; a `removed` decision.
- **Edge / negative cases:**
  - An agent with no grant never opens the shared profile.
  - A grant for a browser that isn't installed is refused.
- **Automation:** Covered by: Rust `pilots/store.rs` (`the_shared_profile_grant_is_per_agent_and_apart_from_capabilities`), `pilots/profiles.rs` (`only_the_visible_browser_gets_the_shared_profile`, `whoever_opens_it_first_holds_it`, `in_use_is_said_calmly`, `a_second_agent_is_told_the_profile_is_in_use_until_the_first_lets_go`), Vitest `PermissionsPanel.test.tsx` ('lets the person give the agent My Nandi browser and take it back', 'offers no shared profile to the headless browser'), `browser-profile.test.ts` | Layer: unit | Gap: no end-to-end run with two agents and a real browser.
- **Needs relay:** no
- **Status:** Shipped

### AG-35 — "Remember how I do this": an agent proposes a playbook

- **What:** An agent drafts a playbook from what the user tells it, or from work it repeats, and saves it only after the user approves.
  - Agents made on this computer (Nandi too) get the host tool `skill_propose` (name, description, when, body, reason) in every run with tools except the email and calendar checks, even with no playbooks yet. Installed pack agents don't, and a call from one is refused.
  - The proposal is checked first (kebab-case name, one-line description and when, 900 words, a reason). A broken one never reaches the user: the agent reads why.
  - The approval card: "Save a new playbook: <name>", why, what it covers, when, and **Show the steps**. For a name it already has: "Change the playbook <name> (replaces the current one)", the old description or when struck through ("Was: …"), and **Show the changes** (a line diff). **Approve** writes `skills/<name>.md` and commits; **Reject** writes nothing.
  - It needs no run plan (not a change for the classifier).
  - From repeated work: when Nandi does a step itself and did the same work itself before, it adds `- [ ] Add a playbook "<name>" (from <n> similar tasks)` to `my-notes/suggestions.md` once (never a dismissed one) and proposes the playbook when the user ticks it or says yes.
- **Where:** `src-tauri/src/services/playbooks.rs` (`propose_spec`, `gate`, `check`, `apply`, `prompt_section`), `services/orchestrator/llm_step.rs` (offer), `services/orchestrator/executor.rs` (gate), `services/orchestrator/tool_calls.rs` (`run_call`), `services/run_plan/classify.rs`, `services/requests/recurrence.rs` (`playbook_hint`), `services/requests/self_step.rs` (`prompt`), `resources/builtin-agents/nandi/agent.md`, `skills/build-a-workflow.md`, `src/components/agent-console/ApprovalRequestCard.tsx`, `PlaybookProposalPreview.tsx`, `playbook-proposal-model.ts`, `src/components/inbox/approval-model.ts`.
- **Preconditions:** None (Nandi is built in).
- **Steps:**
  1. In Nandi's chat: "Remember: I pay bills from Chase, never the card."
  2. Press **Show the steps**, then **Approve**.
  3. Say: "Also remember to save the receipt in my notes." Open **Show the changes**, then **Reject**.
  4. Open Agents › Nandi › Playbooks.
  5. Finish the same kind of work twice with a step Nandi does itself (for example "Pay the Acme bill" two months running), then open My notes › `suggestions.md`.
  6. Tick the line, and tell Nandi.
  7. Open an installed pack agent's chat and ask it to remember something.
- **Expected:**
  - Step 1: an approval card "Save a new playbook: pay-bills" (or similar) with why, what it covers and when; no file yet; no run plan asked for.
  - Step 2: the steps say Chase, never the card; after Approve the agent says it's saved.
  - Step 3: "Change the playbook … (replaces the current one)"; added lines marked +, removed ones -. After Reject the playbook is unchanged.
  - Step 4: the new playbook is listed, with the approved text.
  - Step 5: one line `- [ ] Add a playbook "…" (from 2 similar tasks)`, added once.
  - Step 6: a playbook approval card as in step 1.
  - Step 7: no card; the agent says its playbooks are read-only.
- **Edge / negative cases:**
  - A proposed name like `Pay Bills`, an empty body or a body over 900 words never shows a card.
  - A broken `skills/<name>.md` is never overwritten by a proposal.
  - An email or calendar check never proposes a playbook.
  - Dismissing the suggestion line: it's never added again.
- **Automation:** Covered by: Rust `playbooks.rs` (`a_proposal_is_checked_by_the_domain_rules`, `the_card_sees_what_a_proposal_replaces_from_the_files`, `the_prompt_says_how_to_propose_when_the_agent_can`, `every_proposal_waits_for_the_user`, `an_approved_proposal_is_written_and_committed`, `a_broken_file_of_that_name_is_never_overwritten`, `an_installed_agent_cant_propose`), `orchestrator/middleware_tests.rs` (`a_proposed_playbook_is_written_once_approved`, `a_rejected_proposal_writes_nothing`, `a_proposal_for_an_existing_name_shows_what_it_replaces`, `a_proposal_that_breaks_the_rules_never_reaches_the_user`), `run_plan/classify.rs` (`effects_table`), `requests/recurrence.rs` (`work_nandi_repeats_itself_hints_a_playbook`), `builtin_agents.rs` (shipped playbooks name only real tools), Vitest `playbook-proposal-model.test.ts`, `PlaybookProposalPreview.test.tsx`, `approval-model.test.ts` | Layer: unit, integration | Gap: no check that a model writes the suggestion line only once, or proposes when asked.
- **Needs relay:** no
- **Status:** Shipped

### AG-40 — Weekly retro: Nandi gets better from your corrections

- **What:** Once a week Nandi reads what the person corrected and proposes at most three small changes. The person says yes or no; Nandi then checks whether a change helped. Nandi's code never changes: only gold copy files do.
  - The `weekly-retro` trigger (Fridays at 17:00, on by default, AG-37) runs Nandi with the `weekly-retro` playbook. Its fires are held within six days of the last retro run ("the weekly retro already ran this week").
  - Only that run gets three host tools: `retro_runs` (last week's run records per agent: refused calls, hand-over drafts the person changed, failed and retried runs, each with its run ID, within a budget, quotes marked untrusted), `retro_read` (a local agent's file as it is) and `retro_propose` (agent, target, the whole new file, reason, runs). No other run has them.
  - A proposal targets one agent's `skill`, `playbook:<name>`, `workflow:<id>` or `trigger:<id>`. It must cite real runs from the last week, and there are at most three a week. It is checked when proposed and again when applied: it may edit text and narrow access, and is refused when it adds a tool, a site or more methods, a secret, a connector or folder, a pilot or a pilot action, drops an approval, changes the sandbox, runtime or AWS, widens delegation, adds a workflow step, changes who does one, drops a step's approval, or changes what starts a trigger. An agent from a signed pack is never changed (the refusal suggests a local copy).
  - The Inbox item (counted, it waits a week): "Nandi suggests a change to <agent>: <reason>", what changes, **Show the change** (the diff), the runs it learned from (each opens Run detail), **Apply** and **No thanks**. It has its own chat (`inbox:retro:<id>`).
  - **Apply** writes the file and makes one gold copy commit "Weekly retro: <reason>" naming the target, the agent and the runs. The file must still be as it was when proposed. Uncommitted edits to it are committed first.
  - After Apply, the item stays a week (not counted) with **Undo**. Undo puts the file back in one more commit ("Undo the weekly retro's change"). A file edited since is never overwritten.
  - The check: the agent's share of decisions that were corrections (a refusal, or a hand-over answered **Changed it**) in the four weeks before the change, against those since. After five decisions since, the verdict is set once. When it is no better and corrections remain, one calm note: "This change to <agent> didn't help", the two rates, **Undo** and **Keep it**.
- **Where:** `src-tauri/src/services/retro/` (`mod.rs`, `records.rs`, `target.rs`, `check.rs`, `tool.rs`, `guard.rs`, `proposals.rs`), `services/domain/retro.rs` (the limits), `services/repos/mod.rs` (`write_definition_text`), `db/migrations/009_retro.sql`, `services/inbox/item.rs` and `tool.rs` (the chat), `services/orchestrator/llm_step.rs` and `tool_calls.rs` (offer, call), `resources/builtin-agents/nandi/skills/weekly-retro.md`, `triggers/weekly-retro.yaml`, `rpc/retro.rs`, `src/components/inbox/RetroCard.tsx`, `retro-model.ts`, `inbox-model.ts`, `src/hooks/useRetro.ts`.
- **Preconditions:** A model. An agent with a week of runs where the person refused calls or changed drafts (for example the Social Manager's hand-overs).
- **Steps:**
  1. Open Triggers and test-fire **Weekly retro**.
  2. Open the Inbox item, then **Show the change** and a run link.
  3. Press **Apply**. Open the agent's file and the gold copy history (Settings › Backup).
  4. Press **Undo**.
  5. Test-fire **Weekly retro** again.
  6. Apply a proposal, then answer five more of that agent's approvals, refusing most.
- **Expected:**
  - Step 1: a Nandi run that reads the week and ends with one line; at most three Inbox items, each for one local agent.
  - Step 2: the diff and the runs it cites.
  - Step 3: the file has the change; one commit with the reason and the run IDs.
  - Step 4: the file is as before, in one more commit; the item leaves the Inbox.
  - Step 5: a test fire runs; the scheduled fire within six days is skipped.
  - Step 6: the note "didn't help" with the rates before and after, **Undo** and **Keep it**.
- **Edge / negative cases:**
  - A proposal that adds a tool, a site or a secret, or targets a pack agent, never reaches the Inbox; one made anyway is refused at Apply and nothing is written.
  - The file changed after the proposal: Apply refuses and writes nothing.
  - A fourth proposal in a week is refused.
  - A morning digest or chat run has no retro tools.
- **Automation:** Covered by: Rust `domain/retro.rs` (`the_text_and_narrowing_are_allowed`, `a_tool_a_site_or_a_secret_is_refused`, `an_approval_a_connector_a_pilot_or_the_sandbox_is_refused`, workflow and trigger tests), `retro/tests.rs` (`a_proposal_that_adds_a_tool_a_site_or_a_secret_is_refused`, `a_signed_pack_agent_is_never_changed`, `apply_then_undo_leaves_the_gold_copy_as_it_was`, `a_new_playbook_goes_on_apply_and_away_on_undo`, `an_edit_since_is_never_overwritten`, `the_check_compares_rates_from_run_records_and_offers_the_undo`, `a_proposal_is_recorded_with_its_diff_and_changes_nothing`, `the_tools_are_only_in_the_weekly_retro_run`), `retro/check.rs`, `retro/records.rs`, `retro/guard.rs`, `db/migrations_tests.rs` (`a_0_2_database_gains_the_retro_proposals_and_keeps_every_row`), `inbox/item.rs` (round trip), Vitest `retro-model.test.ts`, `RetroCard.test.tsx`, `inbox-model.test.ts` | Layer: unit, ui | Gap: no end-to-end run of the trigger with a model.
- **Needs relay:** no
- **Status:** Shipped

### AG-29 — Settings tab: Internet access (allowed sites)

- **What:** "Internet access" with three choices: **No internet** ("This agent can't reach any website…"), **Only these sites** (each site in its own box to change, a remove button, "Add a site" with **Add** or Enter, and a hint about `*.example.com` and ports), and **Any site** (saves `*`, with "This agent can reach any website. Choose this only for agents you trust."). Bad sites say "Use a site name like api.example.com, *.example.com or localhost:8080."; a repeat says "That site's already in the list." A site whose rule is limited to some methods shows e.g. "GET only", and saving keeps that limit. Saved with the rest of the form (AG-15); Revert restores the list.
- **Where:** Agent page → Settings → Internet access. `src/components/agents/NetworkAccess.tsx`, `network-access-model.ts`; saved by `repo_save_agent` (`definition_agent` keeps each host's `methods`).
- **Preconditions:** An agent made here (installed agents are read-only, AG-15).
- **Steps:**
  1. Choose Only these sites, add `api.github.com`, Save.
  2. Change it to `*.github.com`, add `localhost:8080`, remove one, Save.
  3. Choose Any site, Save, then back to Only these sites.
  4. Have the agent fetch a site that isn't listed, then one that is.
- **Expected:** agent.md's `network.allow` matches each save; with Any site it's `- host: '*'` and Settings › Internet access says "Any site". Step 3 brings the earlier list back before saving. Step 4: the first is denied and the second allowed, without restarting the agent.
- **Edge / negative cases:** `a.*.com`, `ex*.com`, `host:port` refused before save; duplicate (any case) refused; a policy's network rules still narrow the list (PO).
- **Automation:** Covered by: `NetworkAccess.test.tsx`, `network-access-model.test.ts`, `useAgentEditor.test.ts` (hosts), `repo_service.rs` (save keeps methods) | Layer: unit | Gap: step 4 is manual.
- **Needs relay:** no
- **Status:** Shipped

### AG-17 — Settings tab: Permissions panel

- **What:** Read-only view of what the agent may do, from its definition: "Tools", each with "Allowed" or "Ask" (built-ins get plain names: Run commands, Read files, Write files, List folders, Ask other agents; others in monospace), "Every runtime tool" when `allowed-tools` is omitted, "Needs approval" (or "Nothing needs approval"), "Network" hosts with methods (or "No network access", Denied), and "Pilots": one switch per declared capability to grant or revoke consent, or "Not on this OS". **Edit in Ask** for local agents.
- **Where:** Agent page → Settings → Permissions. `src/components/agent-console/PermissionsPanel.tsx`; pilot grants via `useSetPilotGrant` (pilots section). The section starts with the policies over the agent and what they change (PO-11); this panel shows what agent.md asks for.
- **Preconditions:** An agent with `allowed-tools`, `requires_approval`, `network.allow`, and a `pilots:` entry.
- **Steps:**
  1. Open Settings and read the panel.
  2. Toggle a pilot capability off and on.
- **Expected:** Levels match agent.md. Pilot toggle persists across reload (machine-local consent); a failure toasts "Couldn't grant the capability" or "Couldn't revoke the capability".
- **Edge / negative cases:** Definition fails to load: destructive alert with the error.
- **Automation:** Covered by: `src/components/agent-console/PermissionsPanel.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### AG-18 — Settings tab: Connectors and shared folders

- **What:** Each installed connector with a health dot (Connected / Not connected), a switch to give it to this agent, and the tools it adds as `<connector>__<tool>` badges (sensitive tools outlined). For the filesystem connector, each configured shared folder has Off / Read / Write (Write only if the folder allows it). Every change is written to agent.md at once (`set_agent_connectors`) with a "Settings saved" toast.
- **Where:** Agent page → Settings → Permissions. `src/components/agent-console/ConnectorsPanel.tsx`, `AgentConnectors` in `AgentSettingsTab.tsx`.
- **Preconditions:** The filesystem connector installed with two folders (one read-only).
- **Steps:**
  1. Switch on filesystem; set folder A to Write and B to Read.
  2. Open agent.md.
- **Expected:** `connectors: [{id: filesystem, folders: [...]}]` with the chosen access. Write is disabled for a read-only folder.
- **Edge / negative cases:**
  - No connectors installed: empty note.
  - Without a `folders:` list the agent gets no shared folders.
  - Unknown folder IDs are refused by `set_agent_connectors` (connectors section).
- **Automation:** Covered by: `src/components/agent-console/ConnectorsPanel.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### AG-19 — Settings tab: Sharing (delegation settings)

- **What:** Outbound ("Can call other agents") and inbound ("Accepts tasks from other agents") toggles, off by default. Turning inbound on asks "Let other Nandis send <name> work?" (Cancel / Turn on) first; once on, shows "Who can send them: the N people you trust." with a **Manage in Network** link, and "Tasks at once" (1–64). Per-capability exceptions (allow / deny / ask; ask = no exception; add by name), the agents it can call grouped by capability (load or "not accepting"), and who can call it. Saved to agent.md via `set_agent_delegation`.
- **Where:** Agent page → Settings → Sharing. `src/components/agent-console/DelegationPanel.tsx`.
- **Preconditions:** None (lists need relay or local providers).
- **Steps:**
  1. Turn on inbound; Cancel; turn on again and click Turn on.
  2. Set Tasks at once to 3; add an exception `finance.risk` = deny.
- **Expected:** After confirm, agent.md has `accepts_delegation: true`, `max_delegation_concurrent: 3`, and the override. The Agents list shows Accepts requests On.
- **Edge / negative cases:** Capability names with spaces or duplicates aren't added; a save error toasts.
- **Automation:** Covered by: `src/components/agent-console/DelegationPanel.test.tsx` | Layer: unit | Gap: none. Overlaps the network/delegation section.
- **Needs relay:** no
- **Status:** Shipped

### AG-20 — Settings tab: Clarification policy

- **What:** Under Sharing, "Clarifications": the agent's clarification policy as requester: "Let providers ask questions" (switch), "Rounds of questions per task: N" (slider 0–5), "Answer automatically" topics, "Always ask me" topics (tag inputs, de-duplicated), "Answer within (seconds)" (10–86,400). Defaults: allow, 2 rounds, no topics, 300 s. **Reset** and **Save policy** apply to this policy only; "Saved to the agent's agent.md" appears after saving. Stored as `clarification:` in agent.md.
- **Where:** `src/components/agents/AgentDelegationTab.tsx`, `src/components/agent-console/ClarificationPolicyEditor.tsx`; commands `get_clarification_policy` / `set_clarification_policy` (`src-tauri/src/rpc/delegation_clarification.rs`).
- **Preconditions:** A local agent.
- **Steps:**
  1. Set rounds to 3, add "formatting" under Answer automatically, set Answer within to 600; click Save policy.
  2. Turn off Let providers ask questions.
- **Expected:** agent.md `clarification:` block matches. With the switch off, the other fields are disabled.
- **Edge / negative cases:** Load failure ("Couldn't load the clarification policy") and save failure ("Couldn't save the clarification policy") each show an alert.
- **Automation:** Covered by: `AgentDelegationTab.test.tsx`, `ClarificationPolicyEditor.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### AG-21 — Settings tab: Advanced (container resources)

- **What:** Collapsed "Advanced": "Container image" (placeholder "Default"), "Memory (MB)" (whole number 16 to 1,048,576) and "CPUs" (> 0, ≤ 256), saved as `image`, `memory_mb`, `cpu`; plus the Memory panel (AG-22).
- **Where:** `AgentSettingsTab.tsx`, `resourceError` in `agent-page-model.ts`.
- **Preconditions:** A local agent.
- **Steps:**
  1. Enter memory `8`; then `2048`; CPU `0.5`; Save.
  2. Restart the container (RT-04) and `docker inspect nandi-agent-<tag>-<id>`.
- **Expected:** `8` shows "Memory must be a whole number between 16 MB and 1 TB." and Save is disabled. After save the container runs with 2048 MB and 0.5 CPU.
- **Edge / negative cases:** CPU `0` or `300`: "CPUs must be more than 0 and at most 256."
- **Automation:** Covered by: `agent-page-model.test.ts` (`resourceError`) | Layer: unit + manual (Docker) | Gap: none for validation.
- **Needs relay:** no
- **Status:** Shipped

### AG-22 — Agent memory panel

- **What:** The entries of the agent's `memory.md`, newest first, with timestamp and a "Run xxxxxxxx" link to the source run; edit (textarea, Save/Cancel) and delete per entry; **Refresh from session** re-runs extraction on the latest run; **Disable memory for this agent** writes `memory_disabled: true` to agent.md ("Memory is off: the agent neither reads nor adds entries.").
- **Where:** Agent page → Settings → Advanced. `src/components/agent-console/AgentMemoryPanel.tsx`; commands `load_agent_memory`, `update_agent_memory`, `delete_agent_memory`, `refresh_agent_memory`, `get_agent_memory_enabled`, `set_agent_memory_enabled` (`src-tauri/src/rpc/memory.rs`); HTTP `POST /api/rpc/{command}`.
- **Preconditions:** A model key (for refresh); an agent with a few runs.
- **Steps:**
  1. Hand-edit `memory.md` to add an entry; open the panel.
  2. Edit an entry and save; delete another.
  3. Click Refresh from session.
  4. Toggle Disable memory.
- **Expected:** The hand-written entry appears (hand edits tolerated). Edits and deletes rewrite `memory.md` (git-staged, `repo://file_changed`). Refresh toasts "Memory refreshed from the latest session" and may add 0–3 entries. Disable writes `memory_disabled: true`; Refresh is disabled while off.
- **Edge / negative cases:**
  - Empty edit can't be saved; unchanged edit just closes.
  - Refresh without a key: "Couldn't refresh memory" toast.
  - Undated entries show "Undated".
- **Automation:** Covered by: `AgentMemoryPanel.test.tsx`, Rust tests in `services/agent_memory.rs` (`memory_md_round_trips`, `parses_hand_edited_memory`, …) | Layer: unit + api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### AG-23 — Delete an agent

- **What:** "Delete <name>?" asks the user to type the agent's name ("Type <name> to confirm"), not its ID. The folder moves to the trash (`~/.nandi/trash`), the removal is staged in git, the container and orchestrator stop, `repo://agent-deleted` fires and the Agents list opens with "Deleted <name>". Run history stays in the DB.
- **Where:** Agent page "⋯" → "Delete agent…", Settings → Danger zone, list row menu. `src/components/agents/DeleteAgentDialog.tsx`; `delete_agent` (`services/agents.rs::delete_agent`, `retire_agent`); HTTP `POST /api/rpc/delete_agent`.
- **Preconditions:** A local agent.
- **Steps:**
  1. Open Delete agent…; type the ID instead of the name; then the name; click Delete.
  2. Try to delete an agent that is running a turn.
  3. Try to delete an agent used by a workflow or trigger.
- **Expected:** Delete is disabled until the typed text equals the name. The folder is gone from `.nandi/agents` and present in the trash. Running agent: "Stop the agent's current run first". Queued requests: "Clear the agent's queue first". Used by a workflow/trigger: refused by `check_removable`.
- **Edge / negative cases:** Marketplace agent via API: "'<id>' is installed from the marketplace. Uninstall it from the Marketplace's Installed tab." (UI hides Delete for them).
- **Automation:** Covered by: Rust tests in `services/agents.rs` | Layer: api | Gap: dialog untested.
- **Needs relay:** no
- **Status:** Shipped

### AG-24 — Container start, stop, restart

- **Duplicate:** tested under RT-04. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### AG-25 — Installed (marketplace) agents: read-only banner, fork and consent gate

- **What:** An installed agent's page shows a banner ("From @<publisher>." or "From the <pack> pack.", "This is a shared agent. Change its rules in Settings › Your rules; fork it to change how it works."), **Your rules** (opens the Settings tab), **Fork**, and, until its requested capabilities are allowed, "Before its first run, review and approve what it can do." with **Review and allow**. Sending a message to it before consent fails with "<name> is a marketplace agent and needs your approval before it runs (…). Open it to review what it can do."
- **Where:** `src/components/marketplace/SharedAgentBanner.tsx`, `ForkDialog.tsx`; `fork_installed_agent` (`rpc/marketplace.rs`); gate `services/marketplace/consent.rs::check` called from `services/chat.rs::submit`.
- **Preconditions:** An agent installed from the Library without allowing it.
- **Steps:**
  1. Open the agent; send a message.
  2. Review and allow; send again.
  3. Fork it with a new ID.
- **Expected:** First send shows the error toast; after allowing, the run starts. The fork is a local, editable agent.
- **Edge / negative cases:** Settings fields are disabled for the installed agent.
- **Automation:** Covered by: Rust tests in `services/marketplace/consent.rs` | Layer: api | Gap: none. Overlaps the Library/marketplace section.
- **Needs relay:** no
- **Status:** Shipped

### AG-26 — `verify --agent` CLI check

- **Duplicate:** tested under RT-24. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

## Agent console (Chat tab)

### CH-01 — Send a message to an agent

- **What:** The Chat tab's composer (Chat mode) sends text to the agent. The user message appears at once (optimistic, client UUID persisted by Rust); the run starts or is queued. Enter sends; Shift+Enter (or IME composition) is a newline; the box grows to 160 px. Placeholder changes to "Queue a message for <name>…" while a run is going. A missing-model notice (`LlmKeyNotice`) shows above the box when no model is usable.
- **Where:** Agent page → Chat. `src/components/agent-console/AgentComposer.tsx`, `src/components/ask/AskBox.tsx`, `AskComposer.tsx`; `sendAgentMessage` (`src/lib/agent-actions.ts`); command `send_message` (`rpc/chat.rs` → `services/chat.rs::submit`); HTTP `POST /api/rpc/send_message`.
- **Preconditions:** Model key (or canned engine).
- **Steps:**
  1. Type "hello" and press Enter.
  2. Send whitespace only.
  3. Via API, send 100,001 characters; send with `clientMessageId: "not-a-uuid"`.
- **Expected:** Message shows immediately; a `runs` row with `source_type user`, priority 10, is created; tokens stream into an agent message; the run ends `completed`. Whitespace: nothing is sent (server: "Message is empty"). Long: "Message is longer than 100000 characters". Bad ID: "Invalid message id".
- **Edge / negative cases:**
  - Unknown agent ID: refused (`require_agent`).
  - After restart the conversation (`agent:<id>`) is still there.
- **Automation:** Covered by: Rust tests in `services/chat.rs` (`client_message_ids_are_used_and_validated`, `llm_engine_streams_loops_on_tool_calls_and_completes`), `src/lib/agent-sync.test.ts` (optimistic send) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### CH-02 — Composer Chat / Ask toggle

- **What:** The agent composer has a Chat / Ask toggle: Chat messages the agent; Ask asks Nandi about this agent (answers inline, plans, or edits handed to Ask's editor via Edit in Ask). Each mode keeps its own draft.
- **Where:** `AskBox.tsx` (`chat` prop, `ask.mode.*`), `AgentComposer.tsx`.
- **Preconditions:** Model key.
- **Steps:** Switch to Ask, ask "what does this agent do?".
- **Expected:** The answer appears inline under the box; no run is added to the agent's history.
- **Edge / negative cases:** Ask behavior itself is covered in the Ask section.
- **Automation:** Covered by: `src/lib/ask.test.ts` | Layer: ui-e2e | Gap: toggle untested.
- **Needs relay:** no
- **Status:** Shipped

### CH-03 — Attachments

- **What:** Attach files or images through the native picker (images filter: png, jpg, jpeg, gif, webp, bmp, svg, heic). Up to 10 per message, 25 MB each. Chips show name and size, with remove (which discards the staged file). Files are staged under `~/.nandi/attachments/staging/<id>` and moved to `~/.nandi/attachments/<runId>/<id>/<name>` when sent. A message may be attachments only (its input is the file names). The webview never passes paths.
- **Where:** Composer attach buttons. `AskComposer.tsx`, `AttachmentChip.tsx`; commands `pick_attachments`, `discard_attachment` (`desktop/attachments.rs`, `rpc/attachments.rs`, `services/attachments.rs`); HTTP `POST /api/rpc/discard_attachment` (no picker over HTTP).
- **Preconditions:** Desktop app.
- **Steps:**
  1. Attach two files; remove one; send.
  2. Attach a 30 MB file.
  3. Pick 12 files at once.
- **Expected:** The sent message shows the remaining chip; the file is under the run's folder; Run Detail Outputs and Files list it. 30 MB file: error toast naming the 25 MB limit. 12 files: only 10 are kept; attach buttons disable at 10.
- **Edge / negative cases:** Cancel the picker: nothing happens. Directories and bad IDs are refused.
- **Automation:** Covered by: Rust tests in `services/attachments.rs` (`stages_resolves_and_commits`, `rejects_bad_ids_and_directories`) | Layer: manual (OS dialog) | Gap: picker untested.
- **Needs relay:** no
- **Status:** Shipped

### CH-04 — Message list

- **What:** The conversation as an activity feed (avatar, "You" or agent name, time). Follows new messages and streamed text; a deep-linked message is scrolled to and ringed. Empty state "Give <name> a task" ("Give <name> a task to get started."). Kinds: user (text and attachment chips), agent (markdown), tool call card, approval card, error (alert).
- **Where:** `src/components/agent-console/MessageList.tsx`, `ChatMessageItem.tsx`.
- **Preconditions:** An agent with history.
- **Steps:**
  1. Open Chat during a streaming reply; scroll up.
  2. Open a link that focuses a message (e.g. from a notification or Inbox).
- **Expected:** Auto-scroll to the end while streaming; focused message centered with a ring. Empty agent messages are hidden until text arrives.
- **Edge / negative cases:** A failed run shows its error as a red alert message (e.g. a turn-limit abort reason).
- **Automation:** Covered by: none directly | Layer: ui-e2e | Gap: no component test.
- **Needs relay:** no
- **Status:** Shipped

### CH-05 — Safe markdown rendering of agent output

- **What:** Agent text renders as GitHub-flavored markdown, but raw HTML is never rendered, images are replaced by "[alt]" (never loaded), links open in the system browser (http/https only; never in the webview), and `nandi://request/<id>` links are allowed for Request cards.
- **Where:** `src/components/agent-console/MarkdownContent.tsx`, `markdown-link-context.ts`.
- **Preconditions:** Canned engine or a model you can make echo text.
- **Steps:** Make the agent reply with `<script>`, `![x](http://evil/x.png)`, a table and `[a](javascript:alert(1))`.
- **Expected:** Script shown as text or dropped; image shows "[x]" and no network request; table renders; the javascript link does nothing.
- **Edge / negative cases:** In the web client, http links open a new tab with `noopener`.
- **Automation:** Covered by: none | Layer: unit (easy to add) | Gap: no test.
- **Needs relay:** no
- **Status:** Shipped

### CH-06 — Streaming and the model loop

- **What:** Each step streams tokens (batched ~50 ms/256 bytes into the DB and `agent://token`), then records planned tool calls and runs them; a step with no tool calls completes the run. The system prompt is agent.md body + `instructions.md` + extras (memory, folders, and today's date with the UTC offset, `today_prompt`). History is rebuilt from the DB each step (every tool call answered: output, "rejected" or "not run"; DeepSeek reasoning replayed; last 200 messages). The model is the agent's `model:`, else the default; if that provider has no key, the first provider with one.
- **Where:** `src-tauri/src/services/orchestrator/` (`llm_step.rs`, `executor.rs`), `services/history.rs`, `src-tauri/src/llm/*`.
- **Preconditions:** Docker + image, model key.
- **Steps:** Follow `docs/developer/e2e-checklist.md` steps 1–9 with the echo-agent.
- **Expected:** As in the checklist: tool call card, result with `hello from nandi`, final answer, run `completed`; one `llm.log` line per model call; everything still there after restart.
- **Edge / negative cases:**
  - No key for any provider: the turn fails with a clear message (`llm_engine_without_a_key_fails_the_turn_clearly`).
  - DeepSeek 429/500/503 are retried twice before output.
- **Automation:** Covered by: Rust tests in `services/chat.rs` (mock DeepSeek), `services/history.rs` | Layer: api + manual (real model) | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### CH-07 — Tool call card

- **What:** A collapsible card per tool call: what it does in plain words ("Read my-notes/responsibilities.md", "Find who is responsible: SOC 1", "Read pull request acme/web#12", "AWS: Batch describe", "Search the web: tokio select", "Read https://docs.rs/…"; Studio's own tools as what they look at, "Agent details: writer", "Look at recent runs", never their `nandi_*` IDs; other tools as words, "Open ticket"), status (Running with spinner, Done, Failed), then under the chevron the raw tool name, arguments and result as formatted JSON (max-height scroll, LTR).
- **Where:** `src/components/agent-console/ToolCallCard.tsx`, `tool-label.ts` (`toolLabel`), `format-json.ts`.
- **Preconditions:** Docker + image, model key.
- **Steps:** Ask the agent to list its workdir; expand the card.
- **Expected:** "Look through the shared folders" (or "Look in <path>"), Running while the call executes; then Done, with `list_dir`, its arguments and result in the details; a tool error shows Failed with the error result.
- **Edge / negative cases:** A call blocked by plan mode or not in `allowed-tools` shows an error result and no approval. A known tool without its detail argument reads plainly ("Read a file").
- **Automation:** Covered by: `tool-label.test.ts` | Layer: unit + ui-e2e | Gap: no component test of the card itself.
- **Needs relay:** no
- **Status:** Shipped

### CH-08 — Approval request card and decisions

- **What:** A tool in `requires_approval` suspends the run (`awaiting_approval`, no tokens used) and shows a card: who asks (when shown outside chat), a plain-words summary (e.g. "Run a command: uname -a"; the demo's connector actions read "Post the review on <url>", "Restart the AWS Batch job <id>", "Redeploy the ECS service <service> in <cluster>"; other connector tools "Use <tool> from <connector>"), countdown ("Expires in N min", refreshed every 30 s), a Studio plan preview when the approval starts a Request, raw tool and arguments under **Details**, and **Approve** / **Reject** (Reject asks "Reject this step?" ("The agent won't do it. It carries on without it, or stops if it can't.", with Keep waiting). Decided cards show Approved / Rejected / Expired badges. The same card and decision work in chat, the agent page banner, Overview, Run Detail Approvals, the Inbox and a Request; deciding in one place clears it everywhere (`agent://approval_resolved`).
- **Where:** `src/components/agent-console/ApprovalRequestCard.tsx`, `decideApproval` (`agent-actions.ts`); command `approve_action` (refused unless the approval belongs to the agent), `list_pending_approvals`; HTTP `POST /api/rpc/approve_action`, `POST /api/rpc/list_pending_approvals`.
- **Preconditions:** Docker + image, model key, `requires_approval: [shell]`.
- **Steps:**
  1. Ask for a shell command; Approve.
  2. Ask again; Reject and confirm.
  3. Decide the same approval twice via API.
- **Expected:** Approve: tool runs, run continues to completion. Reject: call skipped, model told (history says rejected), run continues. Second decision: "This approval request is no longer pending". Buttons disable while a decision is in flight.
- **Edge / negative cases:**
  - Approval ID for another agent via `/agents/{other}/approve`: "Approval not found".
  - Approve stays a single click (no confirm) by design.
  - A tool a policy denies (`tools.deny`) is refused before it's asked about: no card, even when it's in `requires_approval` (PO-14).
- **Automation:** Covered by: Rust tests in `services/chat.rs` (`runs_suspend_for_approval_and_resume`, `rejection_skips_the_tool_and_completes`); `approvals_suite.rs` `approvals_and_refusals_are_on_the_record` over the HTTP API (the run waits, is approved and completes, a second decision is refused, the entry in the decisions file), `a_denied_tool_is_refused_without_asking_first` (a denied tool gets no card) | Layer: api | Gap: card UI untested.
- **Needs relay:** no
- **Status:** Shipped

### CH-09 — Approval expiry

- **What:** Every tool approval expires one hour after it was created (`APPROVAL_TTL_MS`). A sweep every 60 s closes overdue ones as `expired` through the same path as a decision; the run resumes without that call. Escalated clarification questions follow their own deadline instead.
- **Where:** `services/chat.rs::expire_overdue_approvals`, `APPROVAL_SWEEP`; `src-tauri/src/model.rs`.
- **Preconditions:** As CH-08; ability to edit `approvals.expires_at` in `state.db` to speed up.
- **Steps:** Create a pending approval; set its `expires_at` to the past; wait up to 60 s.
- **Expected:** Card shows Expired; `agent://approval_resolved` with status expired; the run continues (or ends) and the model reads that the call expired.
- **Edge / negative cases:** A harness (CLI) approval nobody waits for any more is closed as stale and its run cancelled.
- **Automation:** Covered by: Rust test `an_approval_nobody_decides_expires_and_the_run_goes_on` in `services/orchestrator/middleware_tests.rs` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### CH-10 — Restart behavior for runs and approvals

- **What:** On startup, runs left `running` are marked failed; runs `awaiting_approval` survive and their approvals are re-announced; queued runs drain in order. A panicking run or orchestrator is logged, the agent is marked error in `agent_state`, and the orchestrator restarts (up to 5 times, 1 s apart); after that it stays down until the next command for that agent starts a fresh one.
- **Where:** `services/orchestrator.rs` (`MAX_RESTARTS`, `RESTART_DELAY`), `services/queue.rs`.
- **Preconditions:** Docker + image, model key.
- **Steps:**
  1. Leave an approval pending; quit and relaunch the app.
  2. Kill the app mid-stream; relaunch.
- **Expected:** Step 1: the approval card is still there and approving resumes the run. Step 2: that run shows Failed.
- **Edge / negative cases:** A CLI-runtime run waiting on approval at quit is cancelled (the CLI is gone).
- **Automation:** Covered by: Rust tests `approvals_survive_an_orchestrator_restart` (`services/chat.rs`), `the_queue_survives_a_restart_and_drains_in_order` (`services/queue.rs`), `approvals_suite.rs` `a_crashing_orchestrator_restarts_up_to_the_cap` (five crashes restarted, the sixth left down with the agent in error, the waiting approval kept, approving it starts the agent and the run completes) | Layer: api | Gap: killing the app mid-stream (step 2) is manual.
- **Needs relay:** no
- **Status:** Shipped

### CH-11 — One turn per agent and the queue strip

- **What:** Each agent runs one turn at a time. A message sent while busy becomes a `queued` run with an `agent_queue` row; toast "Queued: the agent will get to it after its current task." The strip above the chat (and on Overview) shows "Working on: …" or "Waiting for approval: …", "+N queued" ("Show queue" opens the queue), and **Stop**. Hidden when idle with nothing queued. Priorities: user 10, critical workflow step 30, workflow step 50, delegation 60, schedule 80, background 100 (triggers: cron 80, file 70, webhook 60, device 50, event 60). The next run is the lowest priority number, then oldest.
- **Where:** `src/components/agent-console/TurnQueue.tsx` (`TurnQueueStrip`); `get_agent_queue`, `get_current_turn` (`rpc/queue.rs`); HTTP `POST /api/rpc/get_agent_queue`, `get_current_turn`; event `agent://queue_changed { agentId, queue, current }`.
- **Preconditions:** Model key; an agent that pauses on approval (easiest way to keep it busy).
- **Steps:**
  1. Trigger an approval-gated call so the run waits.
  2. Send two more messages.
- **Expected:** Strip reads "Waiting for approval: …" and "+2 queued". After the approval, the next run starts automatically (its user message is written then).
- **Edge / negative cases:** A run suspended for approval still blocks the queue.
- **Automation:** Covered by: `TurnQueue.test.tsx`, Rust tests in `services/queue.rs` (`send_during_a_turn_queues_instead_of_starting_a_second`, `the_next_run_starts_by_priority_when_the_turn_ends`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### CH-12 — Queue panel: reorder and remove

- **What:** "Queue" sheet: the current run ("Now" or Awaiting approval, its source, Stop) and each waiting request with source (You, Workflow, Another agent, Scheduled, Schedule, File, Webhook, Device, Event), "Priority N", age, and preview (or attachment names). Drag rows, or Alt+↑/↓ on the handle, to reorder (sets a new priority); the X ("Remove from queue") removes a request (ends it as `cancelled`). With nothing waiting: "Nothing else is waiting."
- **Where:** `TurnQueue.tsx` (`QueuePanel`), `src/lib/queue-order.ts` (`priorityForMove`); `cancel_queued_run`, `reprioritize_queued_run`; HTTP `POST /api/rpc/cancel_queued_run`, `reprioritize_queued_run`.
- **Preconditions:** As CH-11 with 3 queued requests.
- **Steps:** Drag the last row to the top; remove the middle one; approve the current one.
- **Expected:** The moved request runs next; the removed one is Cancelled in history; every change arrives via `agent://queue_changed` without polling.
- **Edge / negative cases:**
  - Approve/Cancel never touch queued runs; `cancel_turn` on a queued run errors "This run is still queued; remove it from the queue instead".
- **Automation:** Covered by: `TurnQueue.test.tsx`, `src/lib/queue-order.test.ts`, Rust `cancel_queued_removes_only_that_run`, `reprioritize_reorders_the_queue`, `approve_and_cancel_never_touch_queued_runs` | Layer: api | Gap: drag gesture untested.
- **Needs relay:** no
- **Status:** Shipped

### CH-13 — Stop the current turn

- **What:** **Stop** (strip, queue panel, Run Detail) cancels the current run: the child task is aborted (dropping the LLM stream or tool call), its pending approvals expire, the run ends `cancelled` and the queue moves on.
- **Where:** `cancelTurn` (`agent-actions.ts`); `cancel_turn` (`services/chat.rs`); HTTP `POST /api/rpc/cancel_turn` with `{agentId, turnId}`.
- **Preconditions:** A long-running or approval-blocked run.
- **Steps:** Click Stop during streaming; then on a run waiting for approval.
- **Expected:** Streaming stops at once; status Cancelled; approval card Expired; `agent://turn_done` (cancelled).
- **Edge / negative cases:** Run ID of another agent: "Run not found for this agent". Invalid agent ID refused.
- **Automation:** Covered by: Rust `cancel_ends_a_suspended_run_and_expires_its_approval` (`services/chat.rs`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### CH-14 — Text-only fallback warning

- **What:** If the agent's container can't start when a step needs tools, the step runs without tools (prompt says so) and `agent://warning { agentId, message: "Tool execution unavailable: …" }` shows as a dismissible banner on the agent page until dismissed or the container runs. Not recorded.
- **Where:** `src/components/agent-console/ContainerControls.tsx` (`AgentWarningBanner`), `AgentView.tsx`; `services/orchestrator.rs`.
- **Preconditions:** Model key; Docker stopped (or image missing).
- **Steps:** Send a message needing a tool.
- **Expected:** Banner with the reason; the agent answers in text only.
- **Edge / negative cases:** A connector that can't start warns the agent the same way.
- **Automation:** Covered by: Rust `agent_answers_text_only_when_its_container_cannot_start` (`services/chat.rs`), `a_connector_that_cant_start_warns_the_agent` (`services/orchestrator/connector_tests.rs`) | Layer: api | Gap: banner UI untested.
- **Needs relay:** no
- **Status:** Shipped

### CH-15 — Turn limits (max_turns, max_cost_usd, context cap)

- **What:** Before each step, `turn_limit` aborts at `max_turns` (default 50), `cost_cap` once the run's cost exceeds `max_cost_usd` (default $5), `token_cap` once the token estimate passes the model's context window (128k DeepSeek, else 200k). The run ends `failed` with the reason (shown in chat and as a dashboard attention item) and `agent://turn_aborted { agentId, runId, middleware, reason }` fires. CLI-runtime runs also count steps against `runtime_config.max_turns` or `max_turns`.
- **Where:** `src-tauri/src/services/orchestrator/middleware.rs`; harness budget in `orchestrator/runtime_router.rs` (`max_turns`, `TurnBudget`).
- **Preconditions:** Model key, Docker + image.
- **Steps:** Set `max_turns: 2` and ask for a task needing several tool calls.
- **Expected:** The run fails after 2 steps with a reason naming `max_turns`; queued work proceeds.
- **Edge / negative cases:** Studio and agents with no definition keep only the three built-in limits.
- **Automation:** Covered by: Rust `the_turn_limit_aborts_the_run_cleanly`, `the_cost_cap_aborts_once_the_run_costs_too_much` (`services/orchestrator/middleware_tests.rs`), middleware tests | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### CH-16 — Plan mode (read_only)

- **What:** `middlewares: [read_only]` blocks write-like tools (`write_file`, `edit_file`, `create_file`, `append_file`, `delete_file`, `remove_file`, `move_file`, `rename_file`, `copy_file`, `make_dir`, `apply_patch`, `shell`, `run_command`, also after a connector prefix). A blocked call gets the result "Blocked by plan mode (read-only): …" and is never offered for approval.
- **Where:** `orchestrator/middleware.rs`.
- **Preconditions:** Model key, Docker + image.
- **Steps:** Add `middlewares: [read_only]`; ask the agent to write a file.
- **Expected:** Tool card Failed with the plan-mode message; no file in the workdir.
- **Edge / negative cases:** Reads (`read_file`, `list_dir`) still work.
- **Automation:** Covered by: Rust `plan_mode_rejects_write_tool_calls`, `read_only_blocks_writes_but_not_reads` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### CH-17 — Verify on stop

- **What:** If a run wrote files and is about to finish without evidence of checking them (a test/check command such as `pytest`, `cargo test`, `pnpm test`, `tsc`, `make`, or re-reading a written file in recent steps), it gets one continuation asking it to verify. Firings are logged as JSON lines to `~/.nandi/local/logs/verify.log`. On by default; off when `middlewares` omits it.
- **Where:** `src-tauri/src/services/orchestrator/verify_on_stop.rs`.
- **Preconditions:** Model key, Docker + image.
- **Steps:** Ask the agent to write a file and stop; watch the run.
- **Expected:** An extra user-role continuation message appears once; `verify.log` has a line.
- **Edge / negative cases:** Fires at most once per run; runs that ran a check after the last write end normally.
- **Automation:** Covered by: Rust tests in `verify_on_stop.rs` (`fires_once_per_run_and_logs_each_firing`, …) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### CH-18 — Compaction of long runs

- **What:** When a run's token estimate nears the context window (`compaction.trigger_tokens`, else window minus a buffer), it ends `compacted` and a child run continues from a model-written summary (merged with the previous one when `iterative_merge`) plus the most recent `preserve_recent_percent` of steps. The child links to the parent (`parent_run_id`, `compaction_summary`); `agent://run_compacted` fires. Compaction failure never kills the run.
- **Where:** `src-tauri/src/services/orchestrator/compaction.rs`; UI in RN-12.
- **Preconditions:** Model key; set a low `compaction.trigger_tokens` (e.g. 2000) to trigger it.
- **Steps:** Run a long task with the low threshold.
- **Expected:** Parent shows Compacted; a child run continues and completes; Approve/Cancel target the child.
- **Edge / negative cases:** Compacting a finished run is refused; a failed summary leaves the run going.
- **Automation:** Covered by: Rust tests in `orchestrator/compaction.rs` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### CH-19 — Deferred tool loading (tool_search)

- **What:** An agent with more than 15 tools starts with the core tools (`shell`, `read_file`, `write_file`, `list_dir`) plus `tool_search`; the model searches (BM25 over name and description, default 5, max 20 results) and matches join the next step's tools (`agent://tools_loaded`). `tool_search` never needs approval. State is per run in memory.
- **Where:** `src-tauri/src/services/orchestrator/tool_search.rs`.
- **Preconditions:** An agent with several connectors (over 15 tools), model key, Docker.
- **Steps:** Ask for something that needs a connector tool.
- **Expected:** Timeline shows a `tool_search` call, then the connector tool call.
- **Edge / negative cases:** After an app restart mid-run, the run restarts from the core tools.
- **Automation:** Covered by: Rust tests in `tool_search.rs` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### CH-20 — Automatic agent memory

- **What:** Before a step, memory.md entries are loaded into the system prompt (capped at about 2,000 tokens, newest kept). After a run, if it took at least 3 steps and the input had a directive ("always", "never", "prefer", "remember", "from now on", …) or used a significant tool, a background model call extracts 0–3 one-sentence facts and appends them with timestamp and run ID. Off with `memory_disabled: true`.
- **Where:** `src-tauri/src/services/agent_memory.rs` (`MemoryMiddleware`).
- **Preconditions:** Model key.
- **Steps:** Tell the agent "From now on always answer in bullet points" in a multi-step task; open the Memory panel afterwards; start a new chat turn.
- **Expected:** A new entry linked to the run; later replies follow it.
- **Edge / negative cases:** Transcript text is treated as data (prompt-injection guard in the extraction prompt).
- **Automation:** Covered by: Rust tests in `agent_memory.rs` (`detects_directives_and_significant_tools`, `prompt_section_keeps_the_newest_within_the_cap`, …) | Layer: unit + manual | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### CH-21 — CLI-runtime agents in chat (Claude Code, Codex, Gemini CLI)

- **What:** An agent with `runtime: claude-code|codex|gemini-cli` runs its turns through that CLI in its harness container; updates are written into the run like native steps, so chat, history and Run Detail look the same. Tool kinds in `requires_approval` wait for approval while the CLI waits (approve runs it, reject or 1 h expiry tells the CLI no). A harness that can't start fails the run unless `runtime_config.fallback: native`.
- **Where:** `src-tauri/src/services/orchestrator/runtime_router.rs`, `src-tauri/src/services/harness/approvals.rs`.
- **Preconditions:** Docker, the harness image built, the CLI's sign-in or key.
- **Steps:** Set `runtime: claude-code`, `requires_approval: [shell]`; ask it to run a command.
- **Expected:** An approval card appears; approving lets the CLI continue; the run completes.
- **Edge / negative cases:** Quit while the CLI waits: the run is cancelled on restart.
- **Automation:** Covered by: Rust tests in `runtime_router.rs` | Layer: manual | Gap: real-CLI runs are manual. Overlaps the models & runtimes section.
- **Needs relay:** no
- **Status:** Shipped

### CH-22 — Delegation task strip (requester)

- **What:** Under the chat header, up to 3 in-flight tasks: for tasks this agent sent, a panel with provider, state, ETA, elapsed, progress, **Query status** (live state report), **Cancel**, open thread, "Questions waiting for your answer: N" with **Answer**, "<member> is offline. The request is queued and will be sent automatically when they're back.", and "Taking much longer than expected (over twice the ETA)." Hidden when tasks end or the task service is down.
- **Where:** `src/components/agents/AgentTaskStrip.tsx`, `src/components/agent-console/TaskStatusPanel.tsx`; commands in `src-tauri/src/rpc/tasks.rs` (`list_tasks`, `get_task`, `cancel_task`, `query_task_status`, `get_task_events`, `get_task_messages`, `get_eta_estimate`); HTTP `POST /api/rpc/list_tasks`, `POST /api/rpc/get_task`, `POST /api/rpc/cancel_task`, `POST /api/rpc/query_task_status`, `get_task_events`, `get_task_messages`, `POST /api/rpc/get_eta_estimate`.
- **Preconditions:** Two agents (or two instances via relay); provider `accepts_delegation: true`; requester allowed outbound.
- **Steps:** Ask the requester to delegate; click Query status; then Cancel.
- **Expected:** Panel appears with state; Query shows "Provider says: <state>, <elapsed> elapsed"; Cancel toasts "Task cancelled." and the panel hides.
- **Edge / negative cases:** Query/Cancel failures toast an error.
- **Automation:** Covered by: `TaskStatusPanel.test.tsx`, `src/components/agents/task-utils.test.ts`, Rust tests in `services/a2a/task.rs` | Layer: api | Gap: none. Overlaps the network/delegation section.
- **Needs relay:** no
- **Status:** Shipped

### CH-23 — Provider state panel and reassign

- **What:** For tasks this agent is serving: who asked, the clarification round, questions awaiting an answer; a Reassign menu hands the task to an online teammate serving the same capability (hidden when nobody else could).
- **Where:** `ProviderStatePanel.tsx`, `ReassignMenu.tsx`; `reassign_task`, `list_reassign_candidates` (`rpc/reassign.rs`); HTTP `POST /api/rpc/reassign_task`, `POST /api/rpc/list_reassign_candidates`.
- **Preconditions:** Relay with two members serving one capability.
- **Steps:** Receive a delegated task; open Reassign; pick a teammate.
- **Expected:** The task moves to the teammate; the panel hides here.
- **Edge / negative cases:** No candidates: menu absent.
- **Automation:** Covered by: none for these components | Layer: manual | Gap: no UI test. Overlaps the network/delegation section.
- **Needs relay:** yes
- **Status:** Shipped

### CH-24 — Clarification and escalation cards

- **What:** In a delegation thread, each provider question shows as a card (question, answer once given, round counter, waiting on a person or the agent). A question escalated to the user (escalate topic, `requires_human`, or an auto answer the model couldn't give) shows an escalation card ("A provider has a question") with countdown: Answer, Allow once (the agent answers once), Allow always (the agent answers this topic from now on; adds to `auto_topics`), or Deny (cancels the task). Decisions are logged to `~/.nandi/local/logs/clarification.log`.
- **Where:** `ClarificationCard.tsx`, `EscalationApprovalCard.tsx` (mounted in `src/components/views/DelegationThreadView.tsx`); `services/a2a/clarification.rs`, `services/a2a/escalation.rs`.
- **Preconditions:** Delegation set up (CH-22); provider asks via `ask_requester`.
- **Steps:** Trigger a question with an escalate topic; click Allow always.
- **Expected:** The agent answers; the topic appears under auto topics in AG-20.
- **Edge / negative cases:** Policy `allow: false` answers "Clarification not allowed" and fails the task; rounds past `max_rounds` cancel the provider task.
- **Automation:** Covered by: `ClarificationCards.test.tsx`, Rust tests in `services/a2a/clarification.rs` | Layer: api | Gap: none. Overlaps the network/delegation section.
- **Needs relay:** no
- **Status:** Shipped

### CH-25 — Pilot consent prompt

- **What:** When a run leaves out a declared, running pilot for lack of consent, `pilot://consent_needed` opens a dialog (from anywhere in the app) listing every capability of the pilot, all of which must be ticked to grant. Dismissing keeps it dismissed for the session.
- **Where:** `src/components/agent-console/PilotConsentPrompt.tsx` (mounted in `src/App.tsx`), `PilotConsentDialog.tsx`.
- **Preconditions:** A pilot running on this OS; an agent declaring it in `pilots:` without consent.
- **Steps:** Send a message that needs the pilot; grant all; retry.
- **Expected:** Dialog appears; after granting, the next run gets the pilot tools; the Permissions panel switches are on.
- **Edge / negative cases:** Dismiss, then trigger again in the same session: no dialog.
- **Automation:** Covered by: `PilotConsentPrompt.test.tsx` | Layer: unit | Gap: none. Overlaps the pilots section.
- **Needs relay:** no
- **Status:** Shipped

### CH-26 — Trust graph

- **What:** Force-directed graph of my agents, agents I call, and agents that call me; edges weighted by calls in the last 30 days. Click a node to highlight its connections; click an inbound edge to set allow / deny / ask.
- **Where:** Network → Sharing → Trust graph. `src/components/agent-console/TrustGraph.tsx`, `trust-graph-model.ts`.
- **Preconditions:** Some delegation history.
- **Steps:** Open the graph; click an inbound edge; set deny.
- **Expected:** The trust rule changes; later requests from that caller are refused.
- **Edge / negative cases:** No history: graph with only my agents.
- **Automation:** Covered by: `TrustGraph.test.tsx`, `trust-graph-model.test.ts` | Layer: unit | Gap: none. Owned by the network section.
- **Needs relay:** no
- **Status:** Shipped

### CH-27 — Live sync: snapshots and agent events

- **What:** The UI holds only Rust snapshots plus event deltas. Listeners register once at boot (IPC, or `/ws` for the web client), then snapshots load (`get_agent_snapshot`, `get_conversation`, `get_dashboard_snapshot`, `get_run_detail`, `get_studio_conversations`); events for a conversation mid-load are buffered and replayed. Events (global names, IDs in payload): `agent://token`, `agent://message`, `agent://tool_call`, `agent://tool_result`, `agent://approval_request`, `agent://approval_resolved`, `agent://run`, `agent://turn_done`, `agent://turn_aborted`, `agent://run_compacted`, `agent://tools_loaded`, `agent://queue_changed`, `agent://warning`, `agent://agents_changed`, `dashboard://activity`, `dashboard://attention`. Each is written to SQLite before it's emitted; reducers are idempotent and a gap triggers a snapshot reload.
- **Where:** `src/lib/agent-sync.ts`, `src/lib/store-reducers.ts`, `src/store/agent-store.ts`; `src-tauri/src/services/events.rs`, `services/snapshots.rs`, `rpc/snapshots.rs`; HTTP `POST /api/rpc/get_agent_snapshot`, `get_conversation`, `get_dashboard_snapshot`, `get_run_detail`, WebSocket `/ws`.
- **Preconditions:** Model key.
- **Steps:**
  1. Open an agent's chat in the desktop app and the same agent in the web client; send from one.
  2. Navigate away mid-run and back.
- **Expected:** Both windows stream the same reply; navigation never stops the run; returning shows the full state without reload.
- **Edge / negative cases:** Duplicate or out-of-order token events are ignored or trigger a resync, never duplicated text.
- **Automation:** Covered by: `src/lib/agent-sync.test.ts`, `src/lib/agent-sync.wave1.test.ts`, `src/lib/agent-sync.wave2a.test.ts`, `src/lib/agent-sync.waveef.test.ts`, `src/lib/store-reducers.test.ts`, Rust tests in `services/events.rs` (write-before-emit) | Layer: unit + api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### CH-28 — Working indicator while the agent answers

- **What:** From the moment a message is sent until the reply starts streaming, the conversation ends with the agent's row: its avatar, its name and "Working on it" followed by dots that go ".", "..", "..." on a loop (a still "..." with reduced motion). After 5 seconds it adds how long the turn has taken since the user's message ("12s"). It stays through tool calls, hides while an approval waits for the user, and comes back once it's decided. The Ask box ("Thinking"), the gateway chat ("The gateway is answering") and a clarification waiting on the other agent ("The agent is answering") use the same dots.
- **Where:** `src/components/agent-console/WorkingDots.tsx` (`WorkingDots`, `WorkingRow`), `working.ts` (`isWorking`, `showsWorking`, `turnStartedAt`), `MessageList.tsx` (`working`); agent chat (`AgentView.tsx`), Studio (`StudioView.tsx`), the companion (`CompanionExpanded.tsx`); `.working-dots` in `src/App.css`.
- **Preconditions:** Model key.
- **Steps:**
  1. In Nandi's chat, send "What do I own?".
  2. Wait without doing anything.
  3. Ask for something that needs approval (a request to a teammate).
- **Expected:** The row appears at once, dots cycle, a seconds count appears after 5 s, tool cards appear above it, and it disappears when the answer's text starts. While the approval card waits, there's no row; after Approve it comes back until the answer streams.
- **Edge / negative cases:** A finished, failed or cancelled run shows no row. The screen reader hears "<agent> is working on your request" (`role="status"`), not the dots.
- **Automation:** Covered by: `src/components/agent-console/working.test.tsx`, `AskStreaming.test.tsx`, `ClarificationCards.test.tsx` | Layer: unit | Gap: the animation itself is manual.
- **Needs relay:** no
- **Status:** Shipped

### CH-29 — Select and copy chat text

- **What:** Messages in every chat (agent chat, Studio, the companion, run detail) can be selected with the mouse and copied with Ctrl+C / Cmd+C. Right-clicking selected text shows Copy and Select All, without Cut or Paste. The rest of the app still doesn't select on drag. Run timelines name tools the same way as the chat ("Save a file", "Approval: Save the draft").
- **Where:** `src/components/agent-console/MessageList.tsx` (`select-text`), `src/lib/context-menu.ts` (`showCopyContextMenu`, `installContextMenuGuard`), `RunTimeline.tsx` (`toolLabel`).
- **Preconditions:** A conversation with a reply.
- **Steps:**
  1. Drag over part of a reply and press Ctrl+C.
  2. Paste it elsewhere.
  3. Right-click the selection.
- **Expected:** The text pastes as selected; the menu has Copy and Select All.
- **Edge / negative cases:** A tool card's header toggles on click and doesn't start a selection.
- **Automation:** Covered by: `context-menu.test.ts` ("offers Copy for selected text that cannot be edited") | Layer: unit | Gap: the selection itself (`manual`).
- **Needs relay:** no
- **Status:** Shipped

### CH-30 — Live work in Ask

- **What:** The Nandi home (CH-34) has a **Live** section with every running or waiting run of any agent, however it started (the person's chat, a trigger, a workflow, a delegation, an email or calendar check), and the runs that finished in the last hour. Rows that wait on the person come first with a "Needs you" mark, then working ones, then finished ones (faded). Each row shows the agent's name and icon, what it's doing now (the plan step it's on, "Step 2 of 5 · …") and how it started ("You asked", the trigger's name, "Workflow: Pay bills", "Asked by Nandi"). No live work: no section at all. The sidebar lists the waiting and working ones under Nandi, after New chat. The home lists only the waiting and working ones. Opening one shows that run as a chat: its messages streaming, its plan card, its approval cards (decided right there, through the same `approve_action` and checks as everywhere else), a status line at the top, **Stop** at the top while it works, and **Details** (Run Detail, for history). Tool calls fold under "Show N steps". A failed run says what happened in one line with **Try again**.
- **Where:** Sidebar → Nandi. `src/components/live-runs/` (`LiveRunsSection.tsx`, `LiveRunView.tsx`, `live-run-model.ts`), `src/components/agent-console/steps-model.ts`, `src/components/agent-console/StopRunButton.tsx`, the `live-run` view. Data: `list_live_runs` (`src-tauri/src/rpc/runs.rs`, `services/runs.rs`, `db::live_runs`) and `get_run_conversation` (`rpc/snapshots.rs`), which reads a run's own conversation whichever it is; `loadRunConversation` in `src/lib/agent-sync.ts` puts it in the store so `agent://*` events stream into it. The list refreshes on `agent://run` and `agent://turn_done` (`runsQueryKeys.all`).
- **Preconditions:** Model key; an agent with a trigger (e.g. a cron trigger) and a tool in its `requires_approval`.
- **Steps:**
  1. Fire the trigger (Automations › Triggers › Run now) so the agent's run waits for an approval.
  2. Open Nandi.
  3. Open the Live row, approve the card.
  4. Start a long run in another agent's chat and press **Stop** in its Live chat.
- **Expected:** The trigger's run is first, marked "Needs you", with the trigger's name. Its chat shows the run's messages, plan card and the approval card; approving there resumes the run, the card shows Approved and the status line follows the plan. Stop ends the other run (asks first when it did outward work). Both rows fade, then leave the list an hour after they end.
- **Edge / negative cases:**
  - A delegated task's run (its own `task:` conversation) and an email check's run open the same way.
  - A run that ended over an hour ago opens from a link but isn't listed.
  - An unknown run ID: "This chat isn't here anymore" with **Back to Nandi**.
  - Ask's own chats (Studio runs) are not in Live; they're in the chat list.
- **Automation:** Covered by: `src/components/live-runs/live-run-model.test.ts` (order, groups, origin, status line, messages), `src/components/agent-console/steps-model.test.ts`, `src/components/layout/nav-tree.test.ts` (sidebar), Rust `services::runs::tests::live_runs_are_the_running_waiting_and_just_finished_without_studio`, `services::snapshots::tests::a_runs_conversation_reads_whichever_it_is` | Layer: unit, api | Gap: no component test for `LiveRunView.tsx`.
- **Needs relay:** no

### CH-31 — Reply into a live chat

- **What:** A live chat has a reply box, also while the agent works. A reply is the next turn in that run's own conversation (the agent's chat for a chat, trigger or workflow run; the task's own conversation for a delegated run; the check's for an email or calendar check), run as that agent with its own grants. Sent while it works, the reply waits in the agent's queue and shows above the box as "<Agent> will read this next: …" until its turn starts. After a failed run the box says "Tell <Agent> what to change…".
- **Where:** `src/components/live-runs/LiveRunView.tsx` (`LiveReply`), `waitingReplies` in `live-run-model.ts`, `replyToRun` in `src/lib/agent-actions.ts`. Data: `reply_to_run` (`src-tauri/src/rpc/chat.rs`, `services/chat.rs`: `reply_target` checks the run exists, isn't Ask's own, and its conversation is its agent's).
- **Preconditions:** Model key; an agent with a trigger.
- **Steps:**
  1. Fire the trigger; open its Live chat while it works.
  2. Type "also check the weekend" and send.
  3. Wait for the run to end.
- **Expected:** The reply shows "<Agent> will read this next: also check the weekend" and a "Queued" toast; when the run ends, the reply starts as the next turn in the same chat, and the status line and Stop follow the new turn. A reply to an ended run starts at once.
- **Edge / negative cases:**
  - A reply to a run of Ask's own chats is refused ("Reply to Ask's chats in Ask").
  - A run whose agent was deleted: the reply is refused with the agent-not-found error, the message is removed.
  - Approvals of the new turn show in the same chat and go through `approve_action` as always.
- **Automation:** Covered by: Rust `services::chat::tests::a_reply_goes_to_the_runs_agent_in_the_runs_own_conversation`, `src/components/live-runs/live-run-model.test.ts` (`waitingReplies`) | Layer: unit | Gap: no component test for the reply box.
- **Needs relay:** no

### CH-32 — Delegated work inside the parent chat

- **What:** When an agent (Nandi, say) delegates to another local agent with `delegate_to_agent`, the parent chat shows the other agent's run as a block named for it, like a person in a group chat: "Researcher is working on it", "… needs you", "… is done" or "… stopped", with its icon and **Open chat** (its own live chat). The block's steps (the delegation call itself, its tool calls and messages) fold under "Show N steps"; its waiting approval cards show open in the block, and approving there answers the child's approval through the same `approve_action` and checks. Until the other agent starts, or when the delegation went to another Nandi, the call shows as its usual tool card. The block is never folded into the parent's own steps.
- **Where:** `src/components/agent-console/DelegationBlock.tsx`, `delegation-model.ts` (which call started which runs), `MessageList.tsx` (every chat), `steps-model.ts`. Data: `list_delegated_runs` (`src-tauri/src/rpc/runs.rs`, `services/runs.rs`, `db::delegated_runs`): the runs with source `agent` asked by the parent's agent (its ID or its card ID on this Nandi) that started while the parent run went on. An agent runs one turn at a time and its tool calls one after another, so no new storage is needed. The child's messages come from `get_run_detail` and refresh on its events.
- **Preconditions:** Model key; Nandi with outbound delegation on; another local agent that accepts delegation for a capability and has a tool in its `requires_approval`.
- **Steps:**
  1. In Nandi's chat (or its Live chat in Ask), ask for something only the other agent's capability covers.
  2. Wait for the block, then approve the other agent's card inside it.
- **Expected:** The block reads "<Agent> is working on it", then "<Agent> needs you" with the approval card open. Approving there resumes the other agent's run; the block reads "… is done" when it ends, and Nandi's chat goes on with the result.
- **Edge / negative cases:**
  - Two delegations in one run: each block shows only the runs its call started.
  - A retry on another provider (failover) shows both runs in the same block.
  - A delegation to another Nandi (through the relay) shows only the tool card; no relay call is made to draw it.
- **Automation:** Covered by: `src/components/agent-console/delegation-model.test.ts`, `steps-model.test.ts`, Rust `services::runs::tests::delegated_runs_are_the_ones_its_agent_asked_for_while_it_ran` | Layer: unit | Gap: no component test for `DelegationBlock.tsx`.
- **Needs relay:** no

### CH-33 — The Inbox and Runs point to the live chat

- **What:** An approval in the Inbox has **Open chat** under its card, which opens that run's live chat in Ask (where it can be approved too, in context). In an agent's Runs tab a running or waiting run shows **Live: open the chat** instead of its status; the row itself still opens Run Detail. Run Detail of a running or waiting run has **Live: open the chat** next to Stop. Run Detail stays the history.
- **Where:** `src/components/inbox/Inbox.tsx` (`InboxItemCard`, approval case), `src/components/agents/AgentRunsTab.tsx` (`AgentRunRow`), `src/components/views/RunDetailView.tsx` (header). The `live-run` view (CH-30).
- **Preconditions:** An agent run waiting for an approval.
- **Steps:**
  1. Open the Inbox and press **Open chat** under the approval.
  2. Open the agent's Runs tab and press **Live: open the chat** on the waiting run.
  3. Open the run's Run Detail.
- **Expected:** Each opens the same live chat with the approval card. Run Detail shows **Live: open the chat** while the run goes on, and not after it ends.
- **Edge / negative cases:**
  - Approving in the chat clears the Inbox item (it's the same approval).
  - A finished run's row shows its status and duration as before.
- **Automation:** Covered by: the `live-run` view's model tests (CH-30) | Layer: ui-e2e | Gap: no component test for the links.
- **Needs relay:** no

### CH-34 — The Nandi home: one place to talk

- **What:** One sidebar item, **Nandi** (the sitting bison), replaces the Nandi item and Ask. Like every chat app, the message box (**What should I do?**) is pinned at the bottom, with the example asks ("Try asking": What's waiting for me?, What did my agents do today?, and two things to build) right above it; tapping one sends it. Above them, scrolling: **Live** (what needs you, then what's working; CH-30), then **Recent**: the last 10 chats with any agent, newest first. A chat can be pinned (the pin button) to stay in Recent; the rest fold under "N older chats". Sending from the composer makes a new chat: its own conversation with Nandi (`chat:<id>`), named by its first message, opened at once. A chat shows its messages as they stream (steps folded), Stop while it works, approval cards decided in place, and a composer for the next turn. Nandi's long-lived conversation from before (`agent:nandi`) is one chat in Recent, "Earlier chats with Nandi", unchanged. **Nandi's settings** opens Nandi's agent page. Home's **Ask Nandi…** and the palette's **New chat** start a Nandi chat too. The sidebar lists New chat, the live work, the five latest chats and My notes under Nandi. Ask's editor stays reachable through **Edit in Ask** on an agent's page; Ask is no longer a sidebar item.
- **Where:** `src/components/nandi/` (`NandiView.tsx`, `nandi-home-model.ts`), `src/store/chat-pins-store.ts` (pins, this computer only), `src/hooks/useChats.ts`, the `nandi` view (`navigation-store.ts`), `nav-tree.ts`. Data: `create_chat`, `send_chat_message` (`src-tauri/src/rpc/chat.rs`, `services/chat.rs`, `chat_agent` refuses a conversation that isn't one of the person's chats), `list_chats` (`rpc/snapshots.rs`, `db::list_chats`), `get_conversation` reads chats. No schema change: a chat is a `conversations` row.
- **Preconditions:** Model key.
- **Steps:**
  1. Open Nandi; type "What's waiting for me?" and send.
  2. Go back to Nandi; send another message.
  3. Pin the first chat; make 10 more chats.
- **Expected:** Each send makes its own chat, named by its message, listed under Recent newest first. The pinned chat stays in Recent; the oldest unpinned ones fold under "older chats". Earlier messages with Nandi are in "Earlier chats with Nandi". Each chat's turns run as Nandi with its own grants.
- **Edge / negative cases:**
  - `send_chat_message` to a task's, a check's or Ask's conversation is refused ("That chat was not found").
  - Pins are kept on this computer only; with storage blocked they last until the app closes.
  - No chats and no live work: Nandi's bison and the question in the middle, the box at the bottom. Nandi's chats show the sitting bison, other agents their own icon. Each chat's one-line preview is plain text (`plainPreview` drops Markdown emphasis, headings, list markers and code marks, and keeps links' text). An empty chat shows its agent and the example asks above the box; they go once it has messages.
- **Automation:** Covered by: `src/components/nandi/nandi-home-model.test.ts` (`plainPreview` too), `NandiView.test.tsx` (examples above the box at the bottom, the bison, the plain preview), `src/components/layout/nav-tree.test.ts`, `Sidebar.test.tsx`, `AppShell.test.tsx` (a new chat with its first message), Rust `services::chat::tests::a_chat_message_goes_only_to_a_chat`, `db::tests::chats_are_new_ones_and_each_agents_own_once_it_has_a_message`, `services::snapshots::tests::studio_and_nandi_threads_are_readable_other_chats_are_not` | Layer: unit, ui | Gap: no component test for `NandiView.tsx`.
- **Needs relay:** no

### CH-35 — Nandi makes agents and automations in a chat

- **What:** In the person's chats with Nandi (the Nandi home's chats and its long chat), "make me an agent that …" or "every Monday …" ends with a card in the chat: "Create the agent “Researcher”" (or "Replace …" when it exists), what it may reach (sites from `network.allow`, connectors, tools and pilots) or "It reaches no sites and no connectors", what a workflow or trigger runs, and why the save would be refused. **Create** approves the save, **No** declines it, **Edit** opens the file in Ask's editor and declines the save (the person saves from the editor). Create is off while the file has problems. The same card shows for Ask's own saves. The model writes the file; the code only builds the card and saves through Ask's save path (validation, the gold copy, events). A paired phone can't approve these saves (they create or change what may run).
- **Where:** `src/components/agent-console/ApprovalRequestCard.tsx`, `SavePreviewSection.tsx`, `save-preview-model.ts`, `src/hooks/useSavePreview.ts`. Data: `preview_save` (`src-tauri/src/rpc/studio.rs`, `services/studio/save.rs` `preview`, writes nothing). Nandi gets the build tools (`nandi_list_guides`, `nandi_get_guide`, `nandi_check_artifact`, `nandi_save_artifact`; `studio::tools::BUILD_TOOLS`) and a short prompt only in the person's chats (`orchestrator/llm_step.rs`), answered on the host (`orchestrator/tool_calls.rs`), and every save waits for approval (`orchestrator/executor.rs`, `studio::save::gate`). `own_devices/desktop_only.rs` keeps the save on the computer.
- **Preconditions:** Model key.
- **Steps:**
  1. In a Nandi chat: "make me an agent that reads arxiv.org every morning and sums up new papers on agents".
  2. Read the card; press **Create**.
  3. Ask again for another agent; press **Edit**.
- **Expected:** The card names the new agent, "May reach: arxiv.org", its tools. Create saves it: it shows in Agents, and Nandi's chat goes on. Edit opens the draft in the editor with nothing saved yet.
- **Edge / negative cases:**
  - An invalid file: the card lists the problems and Create is off; No lets Nandi try again.
  - In an email check or a task's conversation Nandi has no build tools, and a call to one there is not answered.
  - The phone shows the save as "Only on this computer".
- **Automation:** Covered by: `src/components/agent-console/save-preview-model.test.ts`, Rust `services::studio::save::tests::a_preview_says_what_a_save_creates_and_may_reach`, `services::studio::tools::tests::nandi_builds_with_the_guides_the_check_and_the_save_only`, `services::own_devices::desktop_only::tests::risky_approvals_stay_on_the_computer` | Layer: unit | Gap: no live-model test of Nandi writing a file.
- **Needs relay:** no

### CH-36 — @mention another agent

- **What:** In the Nandi home's composer and in a chat's composer, typing `@` lists the person's agents (matching what's typed after it, six at most) as buttons above the box; picking one fills in `@Name `. Sending `@Researcher find …` starts a new chat with Researcher instead of Nandi, with the message after the name; in a chat with Researcher it just continues that chat. The longest matching name wins, and the name must end at a space, `,` or `:`. Those chats show in Live and Recent with that agent's name and icon. A message with no known agent after `@` goes to Nandi as typed. Nandi stays the default.
- **Where:** `src/components/ask/mention-model.ts`, `AskComposer.tsx` (`mentions`), `AskBox.tsx` (`chat.mentions`), `src/components/nandi/NandiView.tsx` (`Composer`). Data: `create_chat` and `send_chat_message` (CH-34); the new chat runs as that agent with its own grants.
- **Preconditions:** A second local agent, Researcher.
- **Steps:**
  1. On the Nandi home type `@res`; pick Researcher; type "find three papers on agents"; send.
  2. In that chat send `@Researcher and summarise them`.
- **Expected:** Step 1 opens a new chat with Researcher (its name and icon in the header and in Recent). Step 2 goes on in the same chat.
- **Edge / negative cases:**
  - `@Writers unite` with an agent named Writer: no match, sent to Nandi as typed.
  - `@Researcher` with no message: sent to the current chat's agent as typed.
- **Automation:** Covered by: `src/components/ask/mention-model.test.ts` | Layer: unit | Gap: no component test for the suggestions.
- **Needs relay:** no

### CH-37 — Stop means stop

- **What:** Stop (the button in a live chat or a chat, Run Detail's Stop, or the person typing just "stop", "stop it", "stop now", "stop everything" or "please stop" in a chat with a run going, also from the phone) ends the run and everything it started: the call in flight (the turn is cancelled), the runs it delegated to other local agents and theirs, and the replies queued behind it in the chat (dropped; the toast says how many). Typed "stop" sends nothing. If the agent's visible browser still has pages open, the chat shows "Close the browser too?" with **Close** and **Keep it open**; with no answer in 2 minutes it closes, and the sites it allowed once go with it. Keep it open leaves the window and its sites until it closes. The record (Decisions) says who stopped and which runs.
- **Where:** `src-tauri/src/services/stop.rs` (`is_stop`, `plan`, `stop_run`, `ask_browser`, `answer_browser_question`, `BROWSER_WAIT`), `chat.rs` (`stop_instead` in `send_chat_message` and `reply_to_run`), `own_devices/chat.rs` (from the phone), `pilots/browser.rs` (`close_window`); `stop_run`, `answer_browser_question` (`rpc/chat.rs`); `src/components/agent-console/StopRunButton.tsx`, `src/components/live-runs/BrowserQuestion.tsx`, `src/store/stop-store.ts`, `onStopped` and `stopRun` in `src/lib/agent-actions.ts`.
- **Preconditions:** Model key; Nandi with a browser and a delegating agent.
- **Steps:**
  1. Ask Nandi for something that delegates and browses; while it works, queue a reply; press **Stop**.
  2. Do it again and type "stop" instead.
  3. Leave "Close the browser too?" unanswered for 2 minutes.
- **Expected:** The run, the delegated run and the queued reply all end; "Stopped, with everything it started. 1 waiting message was dropped." The browser question shows; unanswered, the window closes after 2 minutes. Decisions lists the stop with its runs.
- **Edge / negative cases:** "stop the meeting reminder" is an ordinary message. "stop" with nothing going is an ordinary message. **Keep it open** leaves the window and its sites.
- **Automation:** Covered by: Rust `services::stop::tests` (plain stop words, the plan reaching delegated runs and queued replies, the browser closing unanswered, on Close, not on Keep), `services::chat::tests::stop_typed_in_a_chat_with_a_run_going_stops_it_and_sends_nothing`; `src/components/live-runs/BrowserQuestion.test.tsx` | Layer: unit | Gap: no live browser test.
- **Needs relay:** no

## Runs, Run Detail and audit

### RN-01 — Run Detail page and header

- **What:** Title "<agent> · <first line of input, 60 chars> · <time>", started date-time, duration (ticks live while active), status, **Stop** while active, **Re-run** once ended, and an Actions menu (Copy run ID, Open agent, View this agent's runs). Opens on Timeline for an active run and Overview for a finished one (or a requested tab). The tabs use the full width; an ask box sits under every tab and sends its answers to the Summary tab (RN-08). A "Run details" list shows Engine (model label, "No model connected — this run used built-in replies" with **Add a model**, or "Didn't reach a model"), source (You or the workflow/trigger/agent label), duration, tool calls and events.
- **Where:** Agent → Runs → row, Home, a Request step, links `{ kind: 'run-detail' }`. `src/components/views/RunHistoryView.tsx` (wrapper), `src/components/views/RunDetailView.tsx`, `RunDetailTabs.tsx` (`RunMetadata`), `run-logs-model.ts` (`inputHeadline`); `get_run_detail`.
- **Preconditions:** A finished run and an active run.
- **Steps:** Open both; use each actions menu item; open a deleted run ID.
- **Expected:** Default tabs as described; Copy shows the toast "Run ID copied"; unknown run shows "This run no longer exists." with Back; a load error shows "Couldn't load this run." with Try again.
- **Edge / negative cases:** Duration "—" for a run that never ended and isn't active.
- **Automation:** Covered by: `src/components/views/RunDetailView.test.tsx`, `RunHistoryView.test.tsx`, `run-logs-model.test.ts` | Layer: unit + ui-e2e | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-02 — Overview tab: key results

- **What:** Stat cards (duration, tokens with in/out hint, cost, tool calls with failed count, files written), numbers the output states unambiguously (a JSON object/array of numbers or a markdown table with a numeric column) as cards plus a small SVG bar chart with tooltips, else a text summary; then "Generated files (N)" with "View all files".
- **Where:** `src/components/agent-console/RunKeyResults.tsx`, `RunChart.tsx`, `run-detail-model.ts` (`runStats`, `detectNumericData`, `generatedFiles`, `finalOutput`).
- **Preconditions:** A run whose final answer has a markdown table of numbers.
- **Steps:** Open the run's Overview.
- **Expected:** Cards and a chart with one bar per row; prose numbers are not turned into KPIs.
- **Edge / negative cases:** No output: "This run has no output yet." Cost card only with a known model price.
- **Automation:** Covered by: `RunKeyResults.test.tsx`, `run-detail-model.test.ts` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-03 — Timeline tab

- **What:** Start, each message/step in order, then the end (or "running"): green check done, blue pulse running, grey pending, red failed; each step's time to the second and how long it took; the rail turns green behind finished steps. Updates live.
- **Where:** `src/components/agent-console/RunTimeline.tsx` (also used for a Request step), `run-detail-model.ts` (`timelineSteps`).
- **Preconditions:** An active run.
- **Steps:** Open Run Detail during a run.
- **Expected:** New steps append as they happen; the running step has no duration yet.
- **Edge / negative cases:** Failed run: last step red.
- **Automation:** Covered by: `RunTimeline.test.tsx`, `run-detail-model.test.ts` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-04 — Outputs tab

- **What:** "Final output" (the agent's last reply, markdown), "Attachments" the user sent, and "Full conversation (N)" (the message list; a pending approval in it has working Approve / Reject, decided for the run's agent).
- **Where:** `RunDetailTabs.tsx` (`RunOutputs`).
- **Preconditions:** A finished run with an attachment.
- **Steps:** Open Outputs.
- **Expected:** Final reply, attachment chips, transcript.
- **Edge / negative cases:** No reply: "This run has no output yet."
- **Automation:** Covered by: `RunDetailView.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-05 — Files tab

- **What:** Files the run touched (from tool arguments and attachments) with operation (read, write, list, shell, attached), path and size; **Open** and **Show in folder** (desktop only) and **Copy path** (everywhere). Paths under `/agent/workdir` or relative resolve to `~/.nandi/local/work/<agent-id>/…`; `..` escapes are refused; a missing file says "That file isn't on this computer anymore".
- **Where:** `RunDetailTabs.tsx` (`RunFiles`, `FileActions`), `run-detail-model.ts` (`filesTouched`); `resolve_run_file`, `open_run_file` (`rpc/snapshots.rs`, `services/snapshots.rs::workdir_file`); HTTP `POST /api/rpc/resolve_run_file` `{ runId, path }`.
- **Preconditions:** Docker + image, model key; a run that wrote a file.
- **Steps:** Open Files; click Open, Show in folder, Copy path; delete the file on disk and click Open.
- **Expected:** File opens in its default app; folder revealed; clipboard holds the host path; after deletion the error message above.
- **Edge / negative cases:** Shell rows show the command and no file actions. Empty: "No files" ("The run didn't read or write any files.").
- **Automation:** Covered by: `RunDetailView.test.tsx`, Rust tests in `services/snapshots.rs` | Layer: api + manual (OS open) | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-06 — Logs tab

- **What:** One readable line per recorded run event (`run_events`) with time and kind; filter ("Show": All, Model, Tool, Approval, Error, Other); "Search the log" over line and raw JSON; "Events: N"; click a line for raw data; **Copy log** copies everything; "Clear filters" when nothing matches. Analysis citations can highlight an event here.
- **Where:** `RunDetailTabs.tsx` (`RunLogs`, `LogRow`), `run-logs-model.ts` (`logEntry`, `filterLogs`, `LOG_KINDS`).
- **Preconditions:** A finished run with tool calls and an approval.
- **Steps:** Filter Tool; search a tool name; expand a row; Copy log.
- **Expected:** Only tool events; raw JSON shown; clipboard has the whole log.
- **Edge / negative cases:** Unknown event kinds keep their name; copy failure toasts.
- **Automation:** Covered by: `run-logs-model.test.ts`, `RunDetailView.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-07 — Approvals tab

- **What:** Every approval the run asked for, with when it was requested and its outcome; pending ones can be decided here. For a CLI-runtime run that asked nothing, lists what the runtime allowed or denied on its own from the agent's allowed tools and the tools its policies deny (PO-13), with a link to change settings.
- **Where:** `RunDetailTabs.tsx` (`RunApprovals`, `RuntimeDecisions`), `run-logs-model.ts` (`runtimeDecisions`, `ranOnCliRuntime`).
- **Preconditions:** A run with an approval; a CLI-runtime run.
- **Steps:** Approve a pending approval here.
- **Expected:** The card resolves here, in chat and in the Inbox.
- **Edge / negative cases:** No approvals (native run): empty state.
- **Automation:** Covered by: `run-logs-model.test.ts`, `RunDetailView.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-08 — Run summary and questions (analysis)

- **What:** The **Summary** tab (second, after Overview) generates a cached summary on first open (three short paragraphs plus key risks); then free-text questions answered only from the run's own record (events, messages, artifacts), each answer with cited sources (`[event:12]`, `[message:<id>]`) shown as a list; uncited IDs are dropped. Answers are cached per run and normalized question in `local_settings`; active runs aren't cached ("Saved answer" badge on reuse). Without a key: "Add an API key in Settings to use run analysis." with **Open Settings**, which opens Settings › Models & runtimes. An ask box sits under every tab; sending from any tab opens **Summary**, where the answers collect. The summary is not generated until Summary is opened.
- **Where:** `src/components/agent-console/RunSummary.tsx` (`RunSummaryTab`, `RunAskBox`), `RunDetailView.tsx`; `analyze_run`, `summarize_run` (`rpc/analysis.rs`, `services/run_analysis.rs`); HTTP `POST /api/rpc/analyze_run` `{question}`, `POST /api/rpc/summarize_run`.
- **Preconditions:** Model key; a finished run.
- **Steps:** On the Files tab, ask "why did step 3 fail?"; ask the same with different case/spacing.
- **Expected:** Summary opens with the answer and its sources; long citation ids wrap and the page keeps its width; the second ask is served from cache.
- **Edge / negative cases:** Empty question or unknown run refused; question over 2,000 chars refused; context capped at 60,000 chars.
- **Automation:** Covered by: `RunSummary.test.tsx`, `RunDetailView.test.tsx`, Rust tests in `services/run_analysis.rs` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-09 — Stop from Run Detail

- **What:** **Stop** on an active run. If the run wrote, sent, ran a command, delegated, or has a pending approval, it asks first ("Stop this run?": "It has written files, sent something, or is waiting for your approval. Work done so far is kept.", with **Stop run**). A queued run is removed from the queue instead. Toast "Stopped".
- **Where:** `RunDetailView.tsx` (`StopButton`), `stopNeedsConfirm` in `run-logs-model.ts`.
- **Preconditions:** An active run with and without outward calls.
- **Steps:** Stop each.
- **Expected:** Confirm only for the outward one; both end Cancelled.
- **Edge / negative cases:** Outward detection is by tool name pattern (write, send, shell, delegate, …).
- **Automation:** Covered by: `run-logs-model.test.ts`, `RunDetailView.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-10 — Re-run

- **What:** **Re-run** sends the same input to the same agent and stays on the page, with toast "Sent the same input to <agent>" and a **View runs** action. Asks first when the run wrote, sent or delegated ("Send the same input to <agent> again?": "This run wrote files, sent something, or asked another agent. Running it again may do that again.").
- **Where:** `RunDetailView.tsx` (`RerunButton`), `rerunNeedsConfirm`.
- **Preconditions:** A finished run.
- **Steps:** Re-run a read-only run; then one that wrote a file.
- **Expected:** A new run appears in the agent's Runs tab (queued if busy); confirm shown only for the second.
- **Edge / negative cases:** Attachments of the original run are not re-sent (only `run.input`).
- **Automation:** Covered by: `RunDetailView.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-11 — Compaction lineage in Run Detail

- **What:** A run that is part of a compaction chain shows the chain as a tree above the detail (each child indented under the run it continues, current run marked; from the agent's last 200 runs). The tree is labelled "Compaction lineage". A child shows "Continued from run <id>" (link) and a collapsed "Compacted context" with the summary it started from.
- **Where:** `RunHistoryView.tsx` (`RunLineage`, `runLineage`), `RunDetailView.tsx` (`ContinuedFrom`, `CompactedContext`).
- **Preconditions:** A compacted run (CH-18).
- **Steps:** Open the child run; click the parent link.
- **Expected:** Tree shows both runs with statuses Compacted and Completed; navigation works.
- **Edge / negative cases:** Runs with no chain show no tree.
- **Automation:** Covered by: `RunHistoryView.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-12 — "Part of" Request link

- **What:** A run that served a Request (delegated task or Ask plan) shows "Part of a request" linking to the Request page.
- **Where:** `RunDetailView.tsx` (`PartOfRequest`), `request_for_run`.
- **Preconditions:** A run started by a Request.
- **Steps:** Open its Run Detail; click the link.
- **Expected:** The Request page opens.
- **Edge / negative cases:** No Request: nothing shown.
- **Automation:** Covered by: `RunDetailView.test.tsx` | Layer: unit | Gap: none. Overlaps the Requests section.
- **Needs relay:** no
- **Status:** Shipped

### RN-13 — Usage tokens and cost recording

- **What:** Each model call adds input/output tokens and cost to its run (`runs.input_tokens`, `output_tokens`, `cost_usd`, `model`) and writes one JSON line to `~/.nandi/local/logs/llm.log` (model, status, tokens, duration, cost, error; never prompt text). Cost appears only when the model has a known price; DeepSeek uses its peak/off-peak UTC windows.
- **Where:** `src-tauri/src/llm/*`, `src-tauri/src/audit.rs` (`AuditLog::Llm`); shown in RN-02, AG-12, AG-14.
- **Preconditions:** Model key.
- **Steps:** Run one turn; open `llm.log` and the run's Overview.
- **Expected:** One `ok` line per call; token and cost cards match the sum.
- **Edge / negative cases:** Failed calls are logged with `error`.
- **Automation:** Covered by: Rust tests in `src-tauri/src/llm/deepseek.rs` and `audit.rs` | Layer: api + manual | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-14 — Global runs query API

- **What:** Runs across all agents, newest first, filtered by date range, status (`awaitingApproval` or `awaiting_approval`), source type (`user`, `workflow`, `agent`, `scheduled`, `cron`, `file`, `webhook`, `device`, `event`, or `trigger` for any trigger source), agent and text (matches input, summary, source label, agent, workflow and trigger names). Page size default 100, max 200, with total. Studio runs are left out unless asked for by agent ID. Stats for a range: totals, completed, failed, cancelled, tokens, cost, average duration.
- **Where:** `list_all_runs`, `get_run_stats` (`rpc/runs.rs`, `services/runs.rs`); HTTP `POST /api/rpc/list_all_runs`, `get_run_stats` (body `{ range: { from, to } }`). UI consumers: agent Runs tab, Home › All activity (Home section).
- **Preconditions:** Runs from several sources.
- **Steps:** Call with `sourceType=trigger`; with `limit=500`; stats without `from`.
- **Expected:** Only trigger-started runs; at most 200 rows; stats without `from` is a 400.
- **Edge / negative cases:** Unknown status/source values are errors.
- **Automation:** Covered by: Rust tests in `services/runs.rs`, `src/hooks/useRuns.test.ts` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RN-15 — Audit log viewer

- **What:** Settings › Logs with tabs Model calls (`llm`), Internet (`egress`), Triggers, Pilots, Connectors. Decisions aren't a log here: they're in the gold copy, read on Home › Decisions (PO-14). Each shows a table (Time, Agent, a per-tab column, Outcome, Duration, Details), 100 rows per page with Previous page/Next page, "Search this page" with a "Search in" column selector, **Refresh**, and **Download raw log** (all entries as JSONL, `nandi-audit-<kind>-….jsonl`). Entries are metadata only and redacted by the backend (`[redacted]`) and again in the UI; refused/failed outcomes are highlighted.
- **Where:** Sidebar → Settings → Logs. `src/components/views/AuditLogView.tsx`, `src/hooks/useAuditLog.ts`; `list_audit_log` (`rpc/audit.rs`, `src-tauri/src/audit.rs`); files in `~/.nandi/local/logs/`; HTTP `POST /api/rpc/list_audit_log` (body `{ logKind, limit, offset }`).
- **Preconditions:** Some model calls, an egress request and a trigger fire.
- **Steps:** Open each tab; search; page; Download.
- **Expected:** Newest first; downloaded file has every entry, one JSON per line; no keys or prompt text anywhere.
- **Edge / negative cases:** Missing log file = empty list; unknown kind = error; page size capped at 500 server-side. A log Nandi 0.1.0 wrote without the hash chain is kept as `<name>.pre-chain.log` at the first start of a newer build and a new chain starts: the tab shows a calm "Older entries kept" note ("N entries from before the audit chain began (Nandi 0.1.0) are kept in llm.pre-chain.log…"), never "Audit degraded" (`audit/chain.rs`: `a_0_1_0_log_is_set_aside_and_a_new_chain_starts`, `the_first_write_sets_a_0_1_0_log_aside`, `only_unchained_logs_are_set_aside`, `a_kept_copy_is_never_overwritten`; `AuditLogView.test.tsx`).
- **Automation:** Covered by: `AuditLogView.test.tsx`, `src/hooks/useAuditLog.test.ts`, Rust tests in `src-tauri/src/audit.rs` (`redacts_secret_fields_and_text`, `reads_jsonl_newest_first_and_pages`, …) | Layer: unit + api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## Gaps and planned (not testable yet)

- User guide §5 "Turn limits, memory, and self-checks" mentions an agent **Memory tab**; the Memory panel lives under Settings › Advanced, not a tab.
- User guide §5 "Editing an agent" mentions **Sync now** in the account menu and "Start, stop and restart a container agent from the page's ⋯ menu"; the latter is shipped, the former belongs to the repo/account section.
- No status filter for **Compacted** or **Queued** runs on the agent Runs tab (server accepts them).
- No component tests for `AgentsListView.tsx`, `AgentView.tsx`, `AgentSettingsTab.tsx`, `NewAgentDialog.tsx`, `DeleteAgentDialog.tsx`, `ApprovalRequestCard.tsx`, `ToolCallCard.tsx`, `MarkdownContent.tsx`, `ProviderStatePanel.tsx`, `ReassignMenu.tsx`.
- Per-agent Docker workloads (custom images built from a Dockerfile, ports, env, volumes, health checks, job mode): planned in `docs/tasks-todo/task-6-agent-docker-runtimes.md`, not implemented.
- Policies: runs use them (PO-13) and the agent page shows them (PO-11). Approval decisions and policy refusals are in the decisions files, `.nandi/decisions/<yyyy-mm>.md` (PO-14). Still open: `workflow:` targets (see [policies.md](./policies.md) "Gaps and planned"). Raw API keys in harness containers wait for model access through the gateway (`docs/tasks-todo/task-x-enforced-policies.md`, Wave 3).
- Tool-search loaded tools and open clarification rounds are kept in memory only; a restart drops them (documented limitation in `tool_search.rs` and `a2a/clarification.rs`).
