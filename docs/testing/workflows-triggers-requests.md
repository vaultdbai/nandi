# Workflows, triggers, requests, work and the Inbox

Scope: workflow files and the workflow engine, the Automations page (Workflows and Triggers tabs), the workflow graph/builder, steps that wait for a person, workflow approvals, all five trigger sources and their guard rails, the Request page (gateway/delegation tasks and local asks), Work items and the workflow-run project view, the Inbox on Home, the "hold, don't refuse" gateway behaviour, and the per-agent run queue.

Navigation notes (true today): workflows and triggers are tabs of **Automations** (`kind: 'automations', tab: 'workflows' | 'triggers'`), and work waiting on you is in **Home**'s Inbox. There are no `workflows`, `triggers`, `work` or `dashboard` view kinds (`parseView` in `src/store/navigation-store.ts` rejects them). The sidebar's Automations group has two items, Workflows and Triggers (`src/components/layout/nav-tree.ts`). HTTP routes below are under `/api` on the local web server (`get_web_origin`, default `http://127.0.0.1:17463`); external access requires `--server`.

## Workflow definitions and storage

### WF-01 — Workflow YAML format and validation

- **What:** A workflow is a YAML file with `id`, `name`, optional `description`, optional `provides`, and `steps`. Each step has `id`, `agent`, `input` (agent steps) or `step_type: manual` + `instructions` (+ optional `deadline_hours`), and optional `requires_approval`, `critical`, `on_failure: continue`. `{{step.output}}` may only reference earlier steps. Unknown keys are errors.
- **Where:** `~/.nandi/repo/.nandi/workflows/<id>.yaml` or agent-scoped `~/.nandi/repo/.nandi/agents/<agent>/workflows/<id>.yaml`; `src-tauri/src/services/domain/workflow.rs` (`Workflow::from_yaml`, `check`), `src-tauri/src/services/workflows.rs` (`WorkflowDefinition`).
- **Preconditions:** None.
- **Steps:**
  1. Import (WF-07) a valid file with three steps where step 2 uses `{{research.output}}`.
  2. Import variants: empty `name`; no `steps`; an agent step without `agent`; an agent step without `input`; a manual step without `instructions`; `{{later.output}}`; `{{self.output}}`; `{{nope.output}}`; a duplicate step id; an unknown key (`requirs_approval: true`); `provides: ["Bad Id"]`; `provides` listing the same id twice.
- **Expected:** The valid file is accepted. Each invalid file is rejected with a specific message (e.g. "Workflow 'x' has no steps", a cycle error for self/later references, a dangling-reference error for unknown steps, "Duplicate step id", "has an invalid capability id", "lists '...' twice"), shown as a toast.
- **Edge / negative cases:**
  - Input or instructions over 100,000 characters are rejected.
  - Both `requires_approval` and `requiresApproval`, `step_type`/`stepType`, `on_failure`/`onFailure`, `deadline_hours`/`deadlineHours` spellings are accepted.
  - A step naming an agent that doesn't exist is accepted with a log warning (checked at run time, WF-17).
- **Automation:** Covered by: Rust tests in `src-tauri/src/services/domain/workflow.rs` (`parses_and_round_trips`, `detects_cycles_and_dangling_steps`, `rejects_invalid_definitions`, `provides_is_optional_and_round_trips`, `description_is_optional_and_round_trips`), `src-tauri/src/services/workflows.rs` (`parses_the_documented_format`, `rejects_invalid_definitions`, `manual_steps_need_instructions_and_agent_steps_an_agent`) | Layer: unit | Gap: none significant.
- **Needs relay:** no
- **Status:** Shipped

### WF-02 — Workflow files are the gold copy (watcher, projection, read-only pack workflows)

- **What:** Workflow files are mirrored into the `workflows` SQLite table on startup, on every get/run/create, and by the file watcher; hand-edited or pulled files show up live (`workflow://changed`). Invalid files are skipped and logged. Workflows from marketplace packs or vendored agents load read-only.
- **Where:** `src-tauri/src/services/workflows.rs` (`sync_from_disk`), `src-tauri/src/services/repos/workflow_repo.rs`, `src-tauri/src/services/watcher.rs`; frontend invalidation in `src/lib/agent-sync.ts` (`workflow://changed` → `['workflows']`).
- **Preconditions:** App running.
- **Steps:**
  1. With Automations → Workflows open, save a new valid YAML file directly into `~/.nandi/repo/.nandi/workflows/`.
  2. Edit its `name` in a text editor and save.
  3. Save a broken file (`steps:` missing).
  4. Delete the file.
- **Expected:** The list gains the workflow, then shows the new name, ignores the broken file (warning in the log), and drops the workflow when the file is deleted, without a manual refresh.
- **Edge / negative cases:**
  - File name that doesn't match `id` is reported as misnamed and not loaded.
  - Saving a pack's workflow through the Configuration tab fails (read-only).
- **Automation:** Covered by: `syncs_files_into_the_table` (`src-tauri/src/services/workflows.rs`), `keeps_source_text_and_finds_scoped_workflows`, `pack_and_vendored_agent_workflows_load_read_only`, `reports_broken_and_misnamed_files` (`src-tauri/src/services/repos/workflow_repo.rs`) | Layer: unit | Gap: live watcher → UI refresh not tested end to end (ui-e2e).
- **Needs relay:** no
- **Status:** Shipped

## Automations page and workflow list

### WF-03 — Automations page with Workflows and Triggers tabs

- **What:** A single Automations page with two tabs; the sidebar's Workflows/Triggers items and the palette open the right tab.
- **Where:** Sidebar → Automations → Workflows / Triggers; `src/components/views/NavPages.tsx` (`AutomationsPage`), `src/store/navigation-store.ts`, `src-tauri/src/services/palette.rs`.
- **Preconditions:** None.
- **Steps:**
  1. Click Sidebar → Automations → Workflows, then the Triggers tab.
  2. In the command palette, open "Workflows" and then "Triggers".
- **Expected:** The tab switches without leaving the page; the palette results open `{kind:'automations', tab:'workflows'|'triggers'}` with the matching tab selected.
- **Edge / negative cases:** Back/forward navigation restores the tab.
- **Automation:** Covered by: none specific (tabs exercised indirectly in `src/components/views/Workflows.test.tsx`) | Layer: unit (navigation-store) | Gap: no test of the palette's Automations entries.
- **Needs relay:** no
- **Status:** Shipped

### WF-04 — Workflows list

- **What:** Table of every workflow (columns Workflow, Steps, Started by, Last run, Status, Run): name, step count, "Started by" (cron schedules in words, other trigger types by name, or "you, when you click Run"), last run (relative time, duration, "step n of m" while active, or "Never run"), status badge ("Failed at <step>" for a failed run), and a Run button disabled with the tooltip "Already running" while a run is active. "Search workflows" filters by name; skeleton while loading; "Couldn't load workflows" with "Try again"; empty state "No workflows yet" with template/Ask/import actions.
- **Where:** Automations → Workflows; `src/components/views/WorkflowsListView.tsx`, `src/components/automations/automations-model.ts` (`startedByText`, `isActive`, `failedStep`, `currentStepNumber`).
- **Preconditions:** At least one workflow; a cron trigger targeting one workflow.
- **Steps:**
  1. Open the list; read each column.
  2. Type part of a name in Search.
  3. Click Run on an idle workflow; then hover Run while it runs.
  4. Click a workflow name.
- **Expected:** Started by shows e.g. "At 09:00, Monday through Friday"; Run starts a run (toast "Workflow started") and becomes disabled with "Already running"; the name opens the workflow detail page.
- **Edge / negative cases:**
  - No workflows: empty state with "Start from a template", "Describe it in Ask", "or import a workflow file".
  - Search with no matches shows an empty table.
  - Run of a workflow whose agent is missing shows an error toast (WF-17).
- **Automation:** Covered by: `src/components/views/Workflows.test.tsx` ("lists workflows with what starts them and their last run, and runs one"), `src/components/automations/automations-model.test.ts` | Layer: unit (vitest + RTL) | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### WF-05 — New workflow from a template

- **What:** **New workflow** ▾ → "From a template…" opens the dialog "New workflow from a template" listing the bundled templates (Daily summary, Review new files, Approve then send, Two-agent pipeline); the user names it and picks an agent per agent step. The workflow is saved with an id derived from the name (made unique) and its Graph tab opens. With no agents, it offers to install a starter pack instead.
- **Where:** Automations → Workflows → New workflow ▾; `src/components/automations/NewWorkflowMenu.tsx` (`TemplateDialog`), `src-tauri/src/services/workflows/templates.rs`, commands `list_workflow_templates`, `create_workflow_from_template` (`src-tauri/src/commands/workflows.rs`); HTTP `GET/POST /api/workflow-templates`.
- **Preconditions:** At least one agent (for the agent picker).
- **Steps:**
  1. Open the template dialog; choose "Two-agent pipeline".
  2. **Create workflow** stays disabled until both "Agent for “<step>”" selects are chosen; pick agents; click **Create workflow**.
  3. Repeat with the same name.
- **Expected:** File `~/.nandi/repo/.nandi/workflows/<slug>.yaml` is written; toast "Added <name>"; navigation to the workflow's Graph tab. The second create gets a suffixed id.
- **Edge / negative cases:**
  - No agents: dialog shows "Templates need at least one agent. Install a starter pack to get some." with "Install a starter pack" (navigates to starter packs).
  - Backend rejects an agent id that doesn't exist ("Step 'x': ...").
  - Empty name keeps the template's name.
- **Automation:** Covered by: `src/components/views/Workflows.test.tsx` ("starts an empty list from a template, choosing agents by name"), Rust `templates.rs` tests (`every_template_builds_a_valid_two_step_workflow`, `an_agent_step_needs_an_agent`, `approval_and_manual_steps_come_through`, `ids_come_from_the_name_and_stay_unique`) | Layer: unit / api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### WF-06 — Describe a workflow in Studio

- **What:** "Describe it in Ask" opens Ask (the Studio page) on a new chat and sends "Help me build a workflow. Here's what I want it to do: " at once, so the assistant asks what the workflow should do and then drafts the YAML. Ask's own **New** ▾ → **Workflow** does the same.
- **Where:** Automations → Workflows → New workflow ▾ → Describe it in Ask (also on the empty state); `src/components/automations/NewWorkflowMenu.tsx` (`describeInStudio` → `sendToStudio`), `src/components/studio/AskNewMenu.tsx`.
- **Preconditions:** A model configured (for Ask to answer).
- **Steps:** 1. Choose "Describe it in Ask". 2. Answer the assistant with what the workflow should do.
- **Expected:** Ask opens with the message already sent; the drafted workflow can be saved from Ask (AI area).
- **Edge / negative cases:** No model: Ask shows the missing-key notice.
- **Automation:** Covered by: none | Layer: ui-e2e / manual (real LLM) | Gap: no test that the menu calls `sendToStudio`.
- **Needs relay:** no
- **Status:** Shipped

### WF-07 — Import a workflow YAML file

- **What:** "Import a workflow file…" opens a file picker (`.yaml,.yml`); the file is validated and written to the repo, then its page opens. Re-importing identical text is a no-op; a different workflow with an existing id is refused.
- **Where:** Automations → Workflows → New workflow ▾ → Import a workflow file…; `src/components/views/WorkflowImport.tsx` (`useWorkflowImport`), `src/lib/workflow-actions.ts` (`importWorkflowFile`), command `create_workflow`; HTTP `POST /api/workflows` (`{yaml}`); `src-tauri/src/services/repos/workflow_repo.rs` (`create_source`).
- **Preconditions:** A YAML file on disk.
- **Steps:** 1. Import a valid file. 2. Import the same file again. 3. Change a step and import again with the same `id`. 4. Import an invalid file.
- **Expected:** (1) toast "Added <name>", file written, detail page opens; (2) succeeds silently with no change; (3) error "already exists"; (4) validation error toast.
- **Edge / negative cases:** Picking the same file twice still fires (input is reset); a step naming a missing agent is accepted with a warning.
- **Automation:** Covered by: `src/components/views/Workflows.test.tsx` (indirect), Rust `workflow_repo.rs` tests | Layer: api | Gap: no test for the "different workflow, same id" refusal through the command.
- **Needs relay:** no
- **Status:** Shipped

## Workflow detail page

### WF-08 — Workflow header and menu

- **What:** Header with name, latest-run status badge ("Failed at <step>" when failed), subtitle "Steps: N · Started by … · Last run …", **Edit in Ask**, **Run** (disabled while a run is active), and a ⋯ menu ("More for <name>") with **Add a trigger…** (opens New Trigger prefilled with this workflow) and **Copy workflow ID** (toast "Workflow ID copied"). A deleted workflow shows "This workflow isn't here any more." with "Back to Automations".
- **Where:** Automations → Workflows → <workflow>; `src/components/views/WorkflowDetailView.tsx` (`WorkflowHeader`).
- **Preconditions:** A workflow exists.
- **Steps:** 1. Open a workflow. 2. Click Run. 3. ⋯ → Copy workflow ID. 4. ⋯ → Add a trigger…. 5. Delete the YAML file while the page is open.
- **Expected:** Run starts a run; Copy workflow ID copies the id and toasts "Workflow ID copied"; Add a trigger… opens the dialog with target type Workflow and this workflow selected; deleting the file shows the "gone" empty state.
- **Edge / negative cases:** The header no longer shows the file path.
- **Automation:** Covered by: `src/components/views/Workflows.test.tsx` | Layer: unit | Gap: Copy ID / Add a trigger not asserted.
- **Needs relay:** no
- **Status:** Shipped

### WF-09 — Overview tab

- **What:** Cards for: "What it does" (`description`, or "No description yet." with "Add one in Ask"), "Last 30 days" health ("<n> runs: <n> done, <n> failed, <n> waiting on you", "about <duration> each"), "Steps" (agent per step, "Done by a person" for manual, "asks you first" for approval steps), "Started by" (each trigger with "next: <time>" and a "Disabled" marker, plus **Add a trigger…**; "Nothing starts it on its own. It runs when you click Run." if none), and "Recent runs" (each opens the run's project view; "Run now" when empty; "All runs" switches to Runs).
- **Where:** Workflow → Overview tab (default); `src/components/views/WorkflowDetailView.tsx` (`OverviewTab`, `StarterRow`), `automations-model.ts` (`workflowHealth`, `triggersFor`).
- **Preconditions:** A workflow with past runs and a trigger.
- **Steps:** 1. Open the workflow. 2. Read each card. 3. Click a trigger row. 4. Click a recent run.
- **Expected:** Trigger row opens the trigger page; run row opens the workflow-run project view (WF-27).
- **Edge / negative cases:** Runs older than 30 days are excluded from health; a workflow with no runs in 30 days shows "No runs in the last 30 days.".
- **Automation:** Covered by: `src/components/views/Workflows.test.tsx` ("opens on Overview: what it does, steps by agent, and recent runs"), `automations-model.test.ts` (`workflowHealth`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### WF-10 — Graph tab (workflow canvas)

- **What:** A React Flow canvas with one node per step laid out top to bottom, "sequence" edges between consecutive steps and "data" edges for `{{step.output}}` references, and live per-node status from the latest run ("Pending", "Running", "Waiting for your approval", "Waiting on a person", "Success", "Failed", "Skipped"). Nodes aren't draggable. A screen-reader list describes the dependencies.
- **Where:** Workflow → Graph tab; `src/components/workflow-builder/WorkflowCanvas.tsx`, `graph-model.ts` (`layoutGraph`, `applyWorkflowEvent`), `useWorkflowGraph.ts`; command `get_workflow_graph` (`src-tauri/src/commands/workflow_builder.rs`, `src-tauri/src/services/workflow_graph.rs` `build_graph`); HTTP `GET /api/workflows/{id}/graph`.
- **Preconditions:** A workflow with a data reference.
- **Steps:** 1. Open Graph. 2. Run the workflow and watch nodes. 3. Let it fail at a step.
- **Expected:** Edges match the YAML; nodes change status live from `workflow://*` events; steps after a failure show as skipped once the run ended.
- **Edge / negative cases:** Graph load error shows a message; a step added since the last run shows pending.
- **Automation:** Covered by: `src/components/workflow-builder/WorkflowCanvas.test.tsx` ("renders a node per step and an edge per dependency", "updates node status live from workflow events"), `graph-model.test.ts`, Rust `workflow_graph.rs` tests (`builds_nodes_and_edges_from_the_definition`, `takes_statuses_from_the_latest_run`, `unreached_steps_of_an_ended_run_are_skipped_and_new_steps_pending`, `graph_and_yaml_round_trip_through_the_repo`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### WF-11 — Step details panel

- **What:** Clicking a node opens a resizable, closable side panel (Graph tab only) with: agent (or "Done by a person"), status, start time, duration, a missing-agent warning, Approve/Reject or Mark done when the latest run waits on this step, **View run** (opens the agent run), "Message sent", the output (markdown), the error, and a collapsible "Details" ("Step ID", "Uses output of", "On failure": "Stop the workflow" or "Continue").
- **Where:** Workflow → Graph → click a node; `src/components/workflow-builder/StepDetailsPanel.tsx`.
- **Preconditions:** A workflow with at least one finished run.
- **Steps:** 1. Click a step node. 2. Drag the divider. 3. Click View run. 4. Close with × ("Close step details").
- **Expected:** Panel shows the step's data from the latest run; View run opens Run Detail for that agent run.
- **Edge / negative cases:** Step referencing a deleted agent shows the alert "This step's agent isn't installed. Choose another agent below."
- **Automation:** Covered by: `WorkflowCanvas.test.tsx` ("shows the clicked step in the details panel") | Layer: unit | Gap: approval buttons inside the panel not asserted.
- **Needs relay:** no
- **Status:** Shipped

### WF-12 — Edit a step from the panel

- **What:** "Edit step" lets the user change an agent step's agent (picker by name), "What to tell the agent", and "Ask me first" (requires_approval) and save with "Save step"; the YAML file is rewritten (description and manual steps kept). If a run is going, a note says "A run is going. Your change applies to the next run." Manual steps aren't editable here ("A person does this step. Change its instructions in Ask.").
- **Where:** Workflow → Graph → node → Edit step; `StepDetailsPanel.tsx` (`EditStep`), `graph-model.ts` (`workflowYamlWithStep`), command `save_workflow_yaml`; HTTP `PUT /api/workflows/{id}/yaml`.
- **Preconditions:** A workflow and two agents.
- **Steps:** 1. Click **Edit step**; change the agent and "What to tell the agent", and tick "Ask me first". 2. Click **Save step**. 3. Open the Configuration tab.
- **Expected:** "Saved" check appears; YAML shows the new values and `requires_approval: true`; the graph refreshes.
- **Edge / negative cases:**
  - Save disabled until something changed or with no agent.
  - Saved agent missing from the list appears as "<id> (not installed)".
  - Runs in progress keep the definition they started with.
- **Automation:** Covered by: `WorkflowCanvas.test.tsx` ("edits a step through the YAML, picking its agent by name"), `graph-model.test.ts` ("keeps the description and manual steps (regression: an edit dropped them)") | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### WF-13 — Runs tab and transcript

- **What:** Table of the last 20 runs (columns Started, Started by, Status, Took, Request): when, who started it ("You" / "Trigger: <name>" / "a removed trigger" / "Another Nandi" / "—" when unknown), status ("step n of m", "waiting on you", "Failed at <step>"), duration, and an Open link. Selecting a run shows its step-by-step transcript (input, output, step number, "Done by a person", Open run).
- **Where:** Workflow → Runs tab; `WorkflowDetailView.tsx` (`RunsTab`, `TranscriptCard`, `StepTranscript`), `automations-model.ts` (`runStarter`).
- **Preconditions:** Runs started manually and by a trigger.
- **Steps:** 1. Open the Runs tab. 2. Select a run. 3. Click Open (or **View progress** in the banner).
- **Expected:** Starter column names the trigger; transcript matches the steps; Open goes to the run's project view.
- **Edge / negative cases:** No runs: "No runs yet." with "Run now". A run with no recorded `source` shows "—".
- **Automation:** Covered by: `Workflows.test.tsx` ("shows past runs with who started them and their transcript"), `automations-model.test.ts` (`runStarter`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Partial (Open and **View progress** go to the workflow-run project view `work-item`, not a Request page, until task 14; `runRequestView` in `automations-model.ts`. The button was labelled "Open request" until 2026-10-27)

### WF-14 — Configuration tab (edit the YAML)

- **What:** The workflow file in a monospace editor with **Revert** and **Save**. The draft is kept when switching tabs; an "unsaved" dot shows on the tab; leaving the page with unsaved changes asks first. Save validates (domain) and refuses a changed `id`.
- **Where:** Workflow → Configuration tab; `WorkflowDetailView.tsx` (`ConfigurationTab`), `src/components/workflow-builder/actions.ts` (`saveWorkflowYaml`), `src-tauri/src/services/workflow_graph.rs` (`save_workflow_yaml`); `src/components/layout/MainPane.tsx` (`LeaveGuardDialog`).
- **Preconditions:** A writable (non-pack) workflow.
- **Steps:** 1. Edit the name; switch to Graph and back. 2. Click a sidebar item. 3. Save. 4. Change `id:` and Save. 5. Introduce an unknown key and Save.
- **Expected:** Draft survives tab switch; navigation asks to confirm; Save writes the file and toasts "Workflow saved"; id change errors "The YAML's id 'x' doesn't match workflow 'y'..."; invalid YAML shows the error inline.
- **Edge / negative cases:** Revert restores the file; Save disabled with no change.
- **Automation:** Covered by: `Workflows.test.tsx` ("saves the workflow file, holding navigation while it has changes"), `graph_and_yaml_round_trip_through_the_repo` | Layer: unit | Gap: id-mismatch error not covered in UI tests.
- **Needs relay:** no
- **Status:** Shipped

### WF-15 — Workflow context panel (right sidebar)

- **What:** Removed 2026-10-30. The right sidebar listed a workflow's steps and their latest status, which the workflow's own Overview (Steps) and Graph already show; no other page used it. The sidebar, its title-bar button, View › Toggle Right Sidebar and Cmd/Ctrl+2 are gone.
- **Where:** Workflow page Overview and Graph (`WorkflowDetailView.tsx`).
- **Preconditions:** None.
- **Steps:** 1. Open a workflow. 2. Run it.
- **Expected:** The steps and their status show on the page itself; there is no right panel.
- **Edge / negative cases:** None.
- **Automation:** Covered by: `Workflows.test.tsx` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Removed

### WF-16 — Standalone workflow builder view

- **What:** View kind `workflow-builder` renders the canvas standalone with its own selection and step panel beside it (reachable from palette/Ask navigation).
- **Where:** `src/components/layout/MainPane.tsx` (`workflow-builder`), `WorkflowCanvas.tsx` (standalone mode).
- **Preconditions:** A workflow.
- **Steps:** 1. Navigate to `{kind:'workflow-builder', workflowId}` (palette). 2. Click a node.
- **Expected:** Canvas with the step details panel to the side.
- **Edge / negative cases:** None.
- **Automation:** Covered by: `WorkflowCanvas.test.tsx` (standalone canvas) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## Workflow execution

### WF-17 — Run a workflow (sequential engine)

- **What:** Run checks every agent step's agent exists, inserts a `workflow_runs` row, and a driver advances steps in order: each step renders `{{step.output}}` and is submitted as an ordinary agent run (source `workflow`, label "<workflow> · step n") in that agent's conversation, then waits for `agent://turn_done`. The step output is the run's last non-empty assistant message.
- **Where:** Workflow → Run; command `run_workflow`; HTTP `POST /api/workflows/{id}/run`; `src-tauri/src/services/workflows.rs` (`run_workflow`, `start_run`, `drive`, `start_step`, `finish_step`).
- **Preconditions:** Two agents; a model or the canned engine (`engine` = `"canned"` in `local_settings`).
- **Steps:** 1. Create the `echo-chain` workflow from `docs/developer/workflows.md`. 2. Run it. 3. Open each agent's chat.
- **Expected:** `first` completes before `second` starts; `second`'s input contains `first`'s output; each agent's chat has exactly its step message; run status `completed`; `workflow_runs.context` holds per-step status, input, output, agent run id.
- **Edge / negative cases:**
  - Missing agent: error "Step 'x': ..." before any run is created.
  - An agent step failing fails the run (`workflow://failed`) and later steps don't start.
  - Editing the YAML mid-run doesn't change the run (definition copied at start).
- **Automation:** Covered by: `runs_two_steps_in_order`, `renders_step_outputs` (`src-tauri/src/services/workflows.rs`) | Layer: unit/api | Gap: real LLM path manual.
- **Needs relay:** no
- **Status:** Shipped

### WF-18 — Run source recorded ("Started by")

- **What:** Each run records what started it: Manual (Run button/API), Trigger(<id>), Gateway(<task id>), Task(<task id>), kept in `workflow_runs.context`.
- **Where:** `src-tauri/src/services/workflows.rs` (`WorkflowRunSource`), shown by WF-13/WF-04.
- **Preconditions:** A trigger targeting the workflow.
- **Steps:** 1. Run manually. 2. Test-fire the trigger (TR-16). 3. Open Runs.
- **Expected:** Rows say "You" and "Trigger: <trigger name>".
- **Edge / negative cases:** A deleted trigger shows "a removed trigger".
- **Automation:** Covered by: `automations-model.test.ts` (`runStarter`) | Layer: unit | Gap: backend source persistence not directly asserted.
- **Needs relay:** no
- **Status:** Shipped

### WF-19 — Steps that ask first (workflow approvals)

- **What:** A step with `requires_approval: true` pauses the run before sending (`awaiting_approval`, `workflow://step_awaiting_approval`). Approve resumes the driver; Reject (confirmed in the UI) marks the step rejected and fails the run without messaging the agent. Decisions are available in the workflow banner ("“<step>” (<agent>) needs you: it asks you first."), the step panel, the Inbox (RQ-22), and the API.
- **Where:** Workflow page banner; `src/components/automations/StepActions.tsx` (`StepApprovalButtons`); command `approve_workflow_step`; HTTP `POST /api/workflow-runs/{runId}/approve` (`{stepId, decision}`); `workflows.rs` (`approve_step`).
- **Preconditions:** A workflow with an approval step.
- **Steps:** 1. Run it. 2. Approve from the banner. 3. Run again; Reject → confirm.
- **Expected:** Approve: step runs, run completes. Reject: dialog "Reject “<step>”?" ("The run stops here, and the steps after it don't run.") then run `failed` with error "Step 'x' was rejected".
- **Edge / negative cases:**
  - Approving a step not waiting errors "This step is not waiting for approval".
  - Double approve can't run a step twice (one driver per run).
  - Approval steps have no expiry (wait indefinitely, including across restarts).
- **Automation:** Covered by: `pauses_for_approval` (`workflows.rs`), `Workflows.test.tsx` ("approves a waiting step from the banner, by name") | Layer: api / unit | Gap: reject confirmation dialog not tested.
- **Needs relay:** no
- **Status:** Shipped

### WF-20 — Critical steps and busy agents (queueing)

- **What:** `critical: true` queues the step at priority 30 instead of 50. If the agent is busy, the step waits in the agent's queue; removing it from the queue fails the step and the run.
- **Where:** `workflows.rs` (`start_step`), `src-tauri/src/services/queue.rs`.
- **Preconditions:** An agent busy with a long turn.
- **Steps:** 1. Start a long chat turn on agent A. 2. Run a workflow whose step uses A. 3. Cancel the queued step's run from A's queue.
- **Expected:** Step stays `running` (queued) until A is free; cancelling it fails the workflow run.
- **Edge / negative cases:** A critical step jumps ahead of normal-priority queued work for the same agent.
- **Automation:** Covered by: `steps_queue_behind_a_busy_agent_and_a_cancelled_step_fails_the_run` | Layer: api | Gap: critical priority ordering not asserted.
- **Needs relay:** no
- **Status:** Shipped

### WF-21 — Steps that may fail (`on_failure: continue`)

- **What:** A step with `on_failure: continue` doesn't stop the workflow if it fails; later steps see an empty output.
- **Where:** `workflows.rs` (`continue_on_failure`); shown in the step panel Details ("On failure": "Continue").
- **Preconditions:** A step that will fail (e.g. agent stopped/erroring).
- **Steps:** Run a 2-step workflow where step 1 fails and has `on_failure: continue`.
- **Expected:** Step 1 `failed`, step 2 runs with `{{step1.output}}` empty, run completes.
- **Edge / negative cases:** Without the key, the run fails at step 1.
- **Automation:** Covered by: `a_step_that_may_fail_lets_the_run_continue` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### WF-22 — Runs survive restarts

- **What:** On startup `resume_interrupted` restarts drivers for `running` runs; a step whose agent run was interrupted fails; runs awaiting approval or waiting on a manual step simply keep waiting. One driver per run.
- **Where:** `workflows.rs` (`resume_interrupted`, `spawn_driver`, `driving` set).
- **Preconditions:** A run in progress.
- **Steps:** 1. Start a run with an approval step at step 2; quit after step 1. 2. Relaunch. 3. Quit mid-step and relaunch.
- **Expected:** (1-2) Run still awaiting approval after relaunch; (3) the interrupted step and run are failed.
- **Edge / negative cases:** A paused run stays paused after restart.
- **Automation:** Covered by: `a_manual_step_waits_for_a_person_and_survives_a_restart`, `a_driver_save_never_undoes_a_pause_or_cancel` | Layer: api | Gap: real app restart is manual.
- **Needs relay:** no
- **Status:** Shipped

### WF-23 — Workflow events and live refresh

- **What:** After persisting state the engine emits `workflow://step_started`, `step_awaiting_approval`, `step_done`, `done`, `failed`, `manual_step_ready`, `manual_step_completed`, and the watcher emits `workflow://changed`; the frontend invalidates `['workflow', id]` / `['workflows']`.
- **Where:** `workflows.rs` (event structs), `src/lib/agent-sync.ts`.
- **Preconditions:** None.
- **Steps:** Run a workflow while subscribed to `/api/ws` (or watching the UI).
- **Expected:** Events in order step_started → step_done → … → done, with workflowId/runId/stepId payloads.
- **Edge / negative cases:** Events are not stored in `run_events`.
- **Automation:** Covered by: `workflow_events` helper assertions in `workflows.rs` tests; `src/lib/agent-sync.waveef.test.ts` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### WF-24 — Failed-run banner and Run again

- **What:** When the latest run failed, the workflow page shows "Stopped at “<step>”." with the first line of the error, **View progress** and **Run again**; Run again asks first ("Run the whole workflow again?", "Every step runs again from the start: <steps>.").
- **Where:** Workflow page; `WorkflowDetailView.tsx` (`WorkflowBanner`, `RunAgainButton`).
- **Preconditions:** A failed run.
- **Steps:** 1. Open the workflow. 2. Click Run again → Cancel, then Run again → confirm.
- **Expected:** Confirmation dialog names the steps; confirm starts a new run.
- **Edge / negative cases:** Banner hides once a newer run is not failed.
- **Automation:** Covered by: `Workflows.test.tsx` ("confirms before running a failed workflow again") | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## Steps that wait for a person

### WF-25 — Manual steps (`step_type: manual`)

- **What:** Reaching a manual step suspends the run (`waiting`, step `awaitingInput`) with no driver; instructions are rendered with earlier outputs; a deadline is set from `deadline_hours`; a `manual_steps` row indexes it; `workflow://manual_step_ready` fires and the app shows a native notification "<workflow> needs you". The run can wait for days and across restarts.
- **Where:** `src-tauri/src/services/workflows/manual_steps.rs`, `workflows.rs`, `src/lib/agent-sync.ts` (notification); command `list_pending_manual_steps`; HTTP `GET /api/workflows/manual-steps`.
- **Preconditions:** A workflow with a manual step and `deadline_hours: 48`.
- **Steps:** 1. Run it. 2. Observe the notification and Home Inbox. 3. Call `GET /api/workflows/manual-steps`. 4. Restart the app.
- **Expected:** Run `waiting`; the pending list has runId, workflowName, stepId, rendered instructions, deadline ≈ now+48 h, readyAt; still pending after restart.
- **Edge / negative cases:**
  - Cancelling the run closes the manual step (`cancelled`).
  - The run-time agent check skips manual steps.
  - Overdue deadlines are only displayed; nothing escalates.
- **Automation:** Covered by: `a_manual_step_waits_for_a_person_and_survives_a_restart`, `the_run_check_skips_manual_steps`, `a_cancelled_run_drops_its_manual_step` (`workflows.rs`), HTTP tests in `src-tauri/src/http/manual_steps.rs` | Layer: api | Gap: native notification is manual.
- **Needs relay:** no
- **Status:** Shipped

### WF-26 — Complete a manual step (reply, attachments, Mark done)

- **What:** The manual step card (Inbox) shows "Manual step <step> of <workflow>", instructions, "Waiting since …", "Due …" or "Overdue since …", a "Your reply" box, **Attach a file**, and **Complete step**. The reply (plus absolute paths of attached files under `~/.nandi/attachments/workflow-<run>-<step>/…`) becomes `{{step.output}}` and the run resumes. The workflow page banner ("“<step>” is yours to do.") and step panel offer a lighter **Mark done** with an optional "Note for the next step (optional)".
- **Where:** Home → Inbox; Workflow banner; `src/components/work/ManualStepCard.tsx`, `src/components/automations/StepActions.tsx` (`MarkStepDoneButton`); command `complete_manual_step`; HTTP `POST /api/workflows/{runId}/steps/{stepId}/complete` (`{output, attachmentIds}`).
- **Preconditions:** A run waiting on a manual step.
- **Steps:** 1. Type a reply, attach a file, Complete step. 2. Check the next step's input.
- **Expected:** Toast "Step completed; the workflow carries on"; `manual_steps` row `completed`; next agent step's input contains the reply and the attachment path; `workflow://manual_step_completed` and `step_done` emitted.
- **Edge / negative cases:**
  - Completing a run that isn't waiting: "This run is <status>, not waiting on a person".
  - Wrong step: "This step is not waiting for anyone".
  - An empty body (with or without `Content-Type: application/json`) completes the step with no reply; a body that isn't JSON is a 400 "Invalid request body".
  - Attach button only when the client supports attachments (desktop).
- **Automation:** Covered by: `src-tauri/src/http/manual_steps.rs` tests (`an_empty_body_is_a_completion_with_no_reply`, `wave_ef_routes_work_over_http`), `workflows.rs` manual-step tests | Layer: api | Gap: ManualStepCard UI not unit-tested.
- **Needs relay:** no
- **Status:** Shipped

### WF-27 — Workflow run project view (pause, resume, cancel)

- **What:** Opening a workflow run shows a project page: title, state badge, owner, started, ETA; **Pause** (active), **Resume** (paused runs only), **Cancel** (active or waiting); progress bar "n/m"; next-action line ("Approve Post review for PR Reviewer", "Writer is working on Accounts": agent names and step IDs as words, never raw IDs; a manual step shows its instructions); the graph; a Timeline (run started, step started/done/failed/rejected, run ended); "Coming up" with estimated times or "needs approval"; and an Ask box scoped to the workflow. Pause stops between steps; Cancel stops for good and cancels the step's agent turn.
- **Where:** Workflow → Runs → Open, Overview recent runs, Home Running/Recent, Inbox failed row; view kind `work-item`; `src/components/work/WorkflowProjectView.tsx`, `src/lib/work.ts`; commands `get_work_item`, `pause_work`, `resume_work`, `cancel_work`; HTTP `GET /api/work/{id}`, `POST /api/work/{id}/pause|resume|cancel`.
- **Preconditions:** A run with several steps.
- **Steps:** 1. Open a running run. 2. Pause; wait for the current step to finish. 3. Resume. 4. Cancel.
- **Expected:** Pause → state "Waiting on you" with "Paused: resume when you're ready", next step doesn't start; Resume continues; Cancel → Cancelled, running agent turn cancelled, `work://item_updated` emitted.
- **Edge / negative cases:**
  - Pause/resume of a delegated task errors "Delegated tasks can't be paused; cancel it instead".
  - A run waiting on a manual step or an approval offers no Resume: the backend resumes only `paused` runs (`runControls` in `src/lib/work.ts`).
  - Non-workflow work ids show "Open the delegation" (opens the Request page).
- **Automation:** Covered by: `src/components/work/Work.test.tsx`, `src/lib/work.test.ts` (`runControls`, `stepLabel`, `nextActionMessage`), Rust `src-tauri/src/services/work/tests.rs` (`work_pauses_resumes_and_cancels_workflow_runs`) | Layer: unit / api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### WF-28 — Workflows as capability providers (`provides:`, `{{input}}`)

- **What:** A workflow listing `provides:` registers as a capability provider; a delegated or gateway task routed to it starts a run with `{{input}}`/`{{input.field}}` filled from the task, reports per-step progress, and returns the last step's output; the run's id and step breakdown are recorded on the task.
- **Where:** `src-tauri/src/services/capability/workflow_provider.rs`, `workflows.rs` (`run_for_task`, `run_for_gateway`, `render_task_input`).
- **Preconditions:** Network/gateway set up (see network area).
- **Steps:** Send a task for the capability; watch the Request page steps.
- **Expected:** Task Acknowledged → Working with "step n of m" progress; result is the last output.
- **Edge / negative cases:** A step waiting for approval keeps the task waiting until its deadline.
- **Automation:** Covered by: `a_task_run_fills_the_input_and_returns_the_run`, `a_gateway_run_reports_progress_and_returns_the_last_output`, `renders_task_input` | Layer: api | Gap: two-instance flow manual.
- **Needs relay:** no
- **Status:** Shipped

### WF-30 — A step that asks a colleague

- **What:** A workflow step with `ask:` (`find`, `person` or `capability`, plus `input` and optional `due`) sends one request to a colleague's Nandi and waits for the answer, which becomes `{{step.output}}`. With `find`, the owner approves who it goes to before it's sent. The run waits as `waiting` (step `awaitingInput`), across restarts, until the answer comes back or the request fails or expires; the step's deadline is the request's due date. The workflow editor keeps `ask:` blocks when saving, and their `{{…}}` references follow a renamed step.
- **Where:** `services/domain/workflow.rs` (`AskStep`, `check`), `services/workflows.rs` (`WorkflowAsk`, `start_ask`, `resolve_ask`, `settle_asks`, `watch_asks`), `services/orchestrator.rs` (`delegator`), `services/mod.rs` (starts `watch_asks`), `src/components/workflow-builder/graph-model.ts` (`workflowToYaml`).
- **Preconditions:** Two Nandis on a relay; the other person lists a responsibility (for `find`).
- **Steps:**
  1. Save a workflow whose first step is `ask: {find: "<their topic>", input: "…"}` and whose second uses `{{ask.output}}`. Run it.
  2. Approve "Ask <name>: …".
  3. Quit your Nandi; let them answer; start it again.
- **Expected:**
  - Step 1: the run waits for approval and names the person.
  - Step 2: the run is `waiting`; they get the request.
  - Step 3: within seconds of starting, the step is done with their answer and the next step uses it.
- **Edge / negative cases:**
  - Nobody lists the topic: the step fails with "Nobody on your network lists '<topic>' as theirs".
  - Two targets, none, or an `agent:` too: the file is refused on save.
  - Declined or past its due date: the step fails, or the run continues with `on_failure: continue`.
- **Automation:** Covered by: `workflows.rs` (`an_ask_step_waits_for_the_answer_and_passes_it_on`, `an_ask_that_fails_or_expires_fails_its_step`, `an_ask_by_topic_finds_the_owner_and_asks_first`, `an_ask_step_names_exactly_one_target_and_something_to_ask`), `graph-model.test.ts` ("keeps ask steps") | Layer: unit | Gap: the two-Nandi run hasn't been done; cancelling a run doesn't withdraw a request already sent.
- **Needs relay:** yes
- **Status:** Shipped

### WF-29 — Workflow HTTP and command API

- **What:** Headless access to workflows: list, get (definition + last 20 runs + source), create from YAML, templates, run, approve, graph, save YAML, manual steps.
- **Where:** `src-tauri/src/http/routes.rs` (`GET/POST /api/workflows`, `GET /api/workflows/{id}`, `POST /api/workflows/{id}/run`, `GET/POST /api/workflow-templates`, `POST /api/workflow-runs/{id}/approve`), `src-tauri/src/http/workflow_builder.rs` (`GET /api/workflows/{id}/graph`, `PUT /api/workflows/{id}/yaml`), `src-tauri/src/http/manual_steps.rs`.
- **Preconditions:** Web token (non-receiver routes require it).
- **Steps:** Exercise each route with valid and invalid bodies.
- **Expected:** JSON matching the Tauri commands; invalid bodies → 4xx with message.
- **Edge / negative cases:** `PUT /yaml` with `{}` or non-JSON rejected.
- **Automation:** Covered by: `src-tauri/src/http/workflow_builder.rs` tests, `src-tauri/src/http/manual_steps.rs` tests | Layer: api | Gap: no HTTP test for run/approve routes.
- **Needs relay:** no
- **Status:** Shipped

## Triggers

### TR-01 — Trigger rules as files (create, update, reload)

- **What:** Each trigger is `~/.nandi/repo/.nandi/triggers/<id>.yaml` (`id`, `name`, `enabled`, `source{type,config}`, `target{type,id}`, `input_template`, `priority`, `rate_limit_seconds`), validated by `domain::trigger`, projected into `triggers`, with `trigger_state`/`trigger_fires` for history. Ids are slugs of the name (suffixed `-2`, `-3` if taken). Every change writes the file, commits (`repo://trigger-*`, `trigger://changed`) and re-registers cron jobs/watchers; hand edits are picked up by the watcher.
- **Where:** `src-tauri/src/services/triggers/mod.rs` (`build`, `save`, `reload`), `repo.rs`, `src-tauri/src/services/domain/trigger.rs`; commands `create_trigger`, `update_trigger`, `list_triggers`, `get_trigger`.
- **Preconditions:** None.
- **Steps:** 1. Create a trigger in the UI; open the YAML. 2. Edit `name` by hand. 3. Write a file with a `secret` key in its config.
- **Expected:** File matches the documented format without any secret; UI shows the new name; the file with `secret` is rejected.
- **Edge / negative cases:** Empty name; name over 100 chars; template over 20,000 chars; invalid target id; duplicate explicit id ("already exists").
- **Automation:** Covered by: `rules_are_mirrored_to_yaml_and_loaded_from_the_repo` (`src-tauri/src/services/triggers/tests.rs`), `repo.rs` tests (`parses_the_documented_format`, `round_trips_without_the_secret`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-02 — Triggers list

- **What:** Table with an on/off switch, name with source icon, "When" in words (cron in words with the raw expression in a tooltip; "When its webhook is called", "When files change in …", etc.), what it runs ("Asks <agent>" / "Runs <workflow>" by name), last fired (opens what it started), and a ⋯ menu with **Test…** and **Delete…**. Filter chips by source type (All, Schedule, Calendar, File, Webhook, Device, Event); loading, "Couldn't load triggers" with Try again, and the empty state "No triggers yet".
- **Where:** Automations → Triggers; `src/components/views/TriggersListView.tsx`, `automations-model.ts` (`whenText`, `fireView`).
- **Preconditions:** Triggers of several types.
- **Steps:** 1. Open the list. 2. Click each filter chip. 3. Click a time in the "Last fired" column. 4. Click a name.
- **Expected:** Filters narrow the rows; last-fired opens the agent Run Detail (agent target) or the workflow run project view (workflow target); the name opens the trigger page.
- **Edge / negative cases:** A filter with no matches shows "No triggers of this type."; disabled rows are greyed.
- **Automation:** Covered by: `src/components/views/Triggers.test.tsx` ("lists triggers in words, filters by source, and toggles or tests them") | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-03 — New Trigger dialog

- **What:** A dialog with a tab per source type, the shared form ("Name", source fields, "Run" (target), "Message", "Queue priority", **Advanced**: "Minimum seconds between runs" and "Exact priority (1–100)"). **Create trigger** stays disabled while there are problems (name, target, schedule, path, file events, device id). For webhook/device it then shows "Copy the secret now" with the endpoint and the secret once, and **Done**.
- **Where:** Automations → Triggers → New Trigger; Workflow ⋯ → Add a trigger…; `src/components/triggers/NewTriggerDialog.tsx`, `TriggerForm.tsx`, `src/lib/trigger-form.ts`, `src/lib/trigger-actions.ts`.
- **Preconditions:** An agent or workflow to target.
- **Steps:** 1. Open New Trigger. 2. Try **Create trigger** with empty fields. 3. Fill a webhook trigger and click **Create trigger**.
- **Expected:** Create trigger disabled until valid; toast "Created trigger “<name>”"; webhook shows the "Endpoint" URL, "Try it from a terminal" with "Copy command", and the "Secret (shown once)".
- **Edge / negative cases:** Opened from a workflow, target type Workflow and that workflow are preselected.
- **Automation:** Covered by: `Triggers.test.tsx` ("creates a webhook trigger and shows its secret once"), `src/lib/trigger-form.test.ts` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-04 — Schedule (cron) source

- **What:** Fires at each occurrence of a POSIX cron schedule in local time (croner; `1-5` = Monday–Friday; optional leading seconds field). Payload `{scheduled_at, trigger_id}`. Default priority 80.
- **Where:** `src-tauri/src/services/triggers/cron.rs` (`parse_schedule`, `next_after`, `spawn`).
- **Preconditions:** A target agent.
- **Steps:** 1. Create a cron trigger `* * * * *` targeting an agent. 2. Wait two minutes. 3. Try `*/10 * * * * *` (seconds).
- **Expected:** One run per minute with source `cron`; the trigger's Firings tab lists each; seconds schedule fires every 10 s.
- **Edge / negative cases:** Invalid schedule rejected on save; disabling stops further fires; cron is exempt from the storm guard. A fire while the trigger's last run is still queued or running is recorded as `skipped` ("Skipped: the last run is still queued or running") and starts nothing; the same holds for calendar triggers, but not for a test fire or a calendar handover.
- **Automation:** Covered by: `cron.rs` tests (`parses_standard_and_seconds_schedules`, `weekday_schedules_skip_the_weekend`, `seconds_schedules_tick_every_second`), `a_fast_cron_dispatches_at_most_one_run_per_fire`, `a_schedule_skips_a_fire_while_its_last_run_is_still_going` | Layer: unit / api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-05 — Schedule picker with plain-words preview and next runs

- **What:** "When" offers a **Preset** ("Every 5 minutes", "Every hour", "Every day at 9:00", "Weekdays at 9:00", "Mondays at 9:00") or **Custom** cron ("Cron expression"). The form reads it back in the UI language ("At 09:00, Monday through Friday (your time)") and lists the next 3 run times ("Next: …") from the backend's own parser; an unreadable schedule shows an error and disables Save/Create.
- **Where:** `src/components/triggers/TriggerForm.tsx` (`ScheduleFields`), `automations-model.ts` (`describeSchedule`, cronstrue en/fr/ar/ja), command `preview_cron`; HTTP `POST /api/triggers/cron-preview`.
- **Preconditions:** None.
- **Steps:** In New Trigger → Schedule: 1. Pick each preset. 2. Choose Custom and type `0 9 * * 1-5`. 3. Type `61 * * * *`.
- **Expected:** Words and three upcoming times shown; invalid shows "Nandi can't read this schedule. Check it, or pick a preset." and Create trigger (or Save changes on a trigger page) disabled.
- **Edge / negative cases:** Switching UI language changes the words.
- **Automation:** Covered by: `Triggers.test.tsx` ("won't save a schedule the scheduler can't read"), `cron.rs` (`previews_list_the_next_runs_or_say_why_not`), `automations-model.test.ts` (`describeSchedule`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-06 — File source

- **What:** Watches one folder (must exist on save) with optional file-name glob patterns (case-insensitive on Windows), event kinds (created/modified/deleted), per-path debounce (default 2 s) and recursive option. Payload `{path, event, size, modified_at}`. Form fields "Folder" (desktop has a **Browse…** picker), "File name patterns", "Fire when a file is" (Created / Modified / Deleted), "Debounce (ms)", "Include subfolders". Default priority 70.
- **Where:** `src-tauri/src/services/triggers/file.rs`, `TriggerForm.tsx` (`SourceFields`, `pickFolder`).
- **Preconditions:** An existing folder.
- **Steps:** 1. Create a file trigger on `~/Inbox` with pattern `*.pdf`, events created. 2. Drop `a.pdf` and `b.txt`; save `a.pdf` several times quickly.
- **Expected:** One fire for `a.pdf` per debounced burst; none for `b.txt`; `{{payload.path}}` renders the path.
- **Edge / negative cases:** Non-existent folder rejected at save; folder deleted later logs a warning and keeps the rule.
- **Automation:** Covered by: `the_file_watcher_fires_once_per_debounced_burst`, `file_triggers_need_an_existing_folder`, `file.rs` unit tests | Layer: api | Gap: OS folder picker is manual.
- **Needs relay:** no
- **Status:** Shipped

### TR-07 — Webhook source and receiver

- **What:** `POST /api/triggers/webhook/{id}` with header `X-Trigger-Secret` fires the trigger. The receiver skips the web token, compares the secret in constant time, returns 202 once queued; 401 wrong/missing secret, 404 unknown or disabled, 413 over 256 KB, 429 over 60 requests/minute per trigger, 503 dispatcher backed up. Default priority 60.
- **Where:** `src-tauri/src/services/triggers/webhook.rs`, `mod.rs` (`receive_webhook`), `src-tauri/src/http/triggers.rs`, `src-tauri/src/http/auth.rs` (`is_trigger_receiver`).
- **Preconditions:** A webhook trigger and its secret.
- **Steps:** 1. Run the curl from the trigger page. 2. Repeat with a wrong secret, no secret, a 300 KB body, while disabled, and 61 times in a minute.
- **Expected:** 202 and a run with the JSON payload; then 401, 401, 413, 404, 429.
- **Edge / negative cases:** Reachable from other machines only with `--server`.
- **Automation:** Covered by: `a_webhook_queues_a_run_with_its_source_and_payload` (`triggers/tests.rs`), `webhooks_use_the_trigger_secret_instead_of_the_token` (`src-tauri/src/http/mod.rs`), `auth.rs` receiver tests | Layer: api | Gap: 413/429/503 codes not asserted over HTTP.
- **Needs relay:** no
- **Status:** Shipped

### TR-08 — Webhook/device endpoint panel

- **What:** For webhook/device triggers the page shows the URL (Copy disabled, text "Starting Nandi's local server…", until the local server's origin is known), a ready-to-paste curl sample with the header, and the secret only right after creation or rotation ("Copy secret"); otherwise "The secret is hidden. Generate a new one if you've lost it.".
- **Where:** Trigger page → Settings; `src/components/triggers/trigger-ui.tsx` (`TriggerEndpoint`, `CopyButton`), `automations-model.ts` (`webhookCurl`), command `get_web_origin`.
- **Preconditions:** A webhook trigger.
- **Steps:** 1. Open the trigger. 2. Copy URL and curl. 3. Reload the page.
- **Expected:** URL like `http://127.0.0.1:17463/api/triggers/webhook/<id>`; curl includes `X-Trigger-Secret: <your secret>`; secret not shown after reload.
- **Edge / negative cases:** Before the web server is listening, Copy is disabled.
- **Automation:** Covered by: `Triggers.test.tsx`, `automations-model.test.ts` ("writes a webhook sample with the secret header") | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-09 — Rotate a trigger's secret

- **What:** "Generate a new secret" asks first ("Make a new secret?": "The old secret stops working at once…"), then generates a 256-bit secret, stores it in the keyring and shows it once.
- **Where:** Trigger page → Settings; `src/components/triggers/TriggerDialogs.tsx` (`RotateSecretButton`); command `rotate_trigger_secret`; HTTP `POST /api/triggers/{id}/secret`.
- **Preconditions:** A webhook or device trigger.
- **Steps:** On a webhook trigger's Settings tab: 1. Generate a new secret → Cancel. 2. Generate a new secret → **Make a new secret**. 3. Call the webhook with the old and new secret.
- **Expected:** New secret displayed; old secret → 401, new → 202.
- **Edge / negative cases:** Rotating a cron trigger is not offered.
- **Automation:** Covered by: `Triggers.test.tsx` ("asks before making a new secret, then shows it once") | Layer: unit / api | Gap: old-secret-rejected not asserted.
- **Needs relay:** no
- **Status:** Shipped

### TR-10 — Device source and receiver

- **What:** `POST /api/triggers/device/{deviceId}` with `{event, data}` and `X-Trigger-Secret` fires every enabled device trigger for that device whose secret matches and whose `events` list includes the event (empty = all). Runs are queued even while the agent's container is stopped or busy. Default priority 50.
- **Where:** `src-tauri/src/services/triggers/device.rs`, `mod.rs` (`receive_device`), `src-tauri/src/http/triggers.rs`.
- **Preconditions:** A device trigger with `deviceId: printer-01`, `events: [paper_out]`.
- **Steps:** 1. POST `{event:"paper_out"}`. 2. POST `{event:"jam"}`. 3. POST with a wrong secret / unknown device.
- **Expected:** First fires; second fires nothing; wrong secret 401, unknown device 404.
- **Edge / negative cases:** No device discovery or pairing exists.
- **Automation:** Covered by: `device_events_fire_matching_triggers_even_while_the_agent_is_busy`, `device.rs` (`fires_only_matching_events_with_the_right_secret`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-11 — Event source (chain off other runs)

- **What:** Fires on `agent.turn_done`, `workflow.done` or `workflow.failed` from the internal bus when the payload matches all key/value filters (e.g. `agent_id`, `status`, `source_type`). A trigger never fires on a run it dispatched itself (recorded as `skipped`). Default priority 60.
- **Where:** `src-tauri/src/services/triggers/event.rs` (`translate`, `filter_matches`), `mod.rs` (`watch_bus`), `dispatch.rs`; form `EventFields` ("Event", "Only when" rows of Field / Value, **Add condition**).
- **Preconditions:** Agents A and B.
- **Steps:** 1. New Trigger → Event: `agent.turn_done` with conditions `agent_id = A`, `status = failed`, Run B. 2. Make A fail. 3. Make a trigger whose filter matches its own target's runs.
- **Expected:** B runs after A fails; the self-matching trigger's fire shows `skipped` "Ignored: the run was started by this trigger".
- **Edge / negative cases:** Filters compare snake_case payload fields; non-matching events don't fire.
- **Automation:** Covered by: `event_triggers_chain_off_other_runs_but_not_their_own`, `event.rs` tests | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-12 — Targets and dispatch (agent or workflow)

- **What:** An agent target enqueues a chat run in that agent's conversation with `source_type` = trigger type, `source_id` = trigger id, `source_label` = name, and the trigger's priority (queued behind the current turn). A workflow target starts a run recorded as started by the trigger; the message template is ignored (the form says so). The `trigger_fires` row is written first (`queued`), then updated with the run id or `error`.
- **Where:** `src-tauri/src/services/triggers/dispatch.rs` (`process`, `dispatch`), `TriggerForm.tsx` (`TargetFields`).
- **Preconditions:** An agent and a workflow.
- **Steps:** 1. Test-fire an agent-target trigger. 2. Test-fire a workflow-target trigger. 3. Point a trigger at a deleted agent and fire.
- **Expected:** (1) agent run with source trigger; (2) workflow run with Started by = trigger; (3) fire `error`, `trigger://error` emitted.
- **Edge / negative cases:** A saved target that no longer exists stays selectable in the form (shown by id).
- **Automation:** Covered by: `a_webhook_queues_a_run_with_its_source_and_payload`, `dispatch.rs` tests | Layer: api | Gap: workflow-target dispatch not asserted in `triggers/tests.rs`.
- **Needs relay:** no
- **Status:** Partial (workflow targets take no input; `docs/developer/triggers.md`)

### TR-13 — Message (input) template

- **What:** `input_template` renders `{{payload.a.b}}` (array indices allowed), `{{trigger.name}}`, `{{trigger.id}}`, `{{fired_at}}` (ISO 8601), `{{last_run.output}}` (latest completed run's reply). Missing keys become empty strings and are logged. With no template the agent gets the event as JSON.
- **Where:** `src-tauri/src/services/triggers/template.rs`, `dispatch.rs`.
- **Preconditions:** A webhook trigger targeting an agent.
- **Steps:** 1. Template `Order {{payload.order.id}} at {{fired_at}} ({{trigger.name}})`. 2. Fire with `{"order":{"id":42}}`. 3. Fire with `{}`.
- **Expected:** Agent receives "Order 42 at 2026-…(Name)"; second fire renders an empty id and logs the missing key.
- **Edge / negative cases:** Template over 20,000 chars rejected.
- **Automation:** Covered by: `template.rs` unit tests | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-14 — Priority and rate limit settings

- **What:** "Queue priority" is High (30) / Normal (source default) / Low (100), or "Exact priority (1–100)" under **Advanced** along with "Minimum seconds between runs" (`rate_limit_seconds`). Lower runs sooner in the agent queue.
- **Where:** `TriggerForm.tsx` (`PriorityFields`), `automations-model.ts` (`PRIORITY_VALUES`, `priorityLevel`), `src/lib/trigger-form.ts` (`DEFAULT_PRIORITIES`).
- **Preconditions:** None.
- **Steps:** 1. Set High, save, check YAML `priority: 30`. 2. Set Normal, save. 3. Enter custom 42.
- **Expected:** YAML priority 30; Normal stores the source default; Custom shows "Custom (42)".
- **Edge / negative cases:** A stored value not matching a level reads as Custom.
- **Automation:** Covered by: `automations-model.test.ts` (`priorityLevel`), `trigger-form.test.ts` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-15 — Turn a trigger on or off

- **What:** The switch (list and page) writes `enabled` to the file and reloads; disabled triggers don't fire (webhooks get 404). On failure the switch reverts and a toast says "Couldn't turn the trigger on" / "Couldn't turn the trigger off" with the reason.
- **Where:** List row / trigger header; `src/lib/trigger-actions.ts` (`setTriggerEnabled`); command `set_trigger_enabled`; HTTP `POST /api/triggers/{id}/enable|disable`.
- **Preconditions:** A trigger.
- **Steps:** On a webhook trigger: toggle off (list "On" column or page header), fire the webhook, toggle on, fire again.
- **Expected:** 404 while off; YAML `enabled: false`; 202 after on.
- **Edge / negative cases:** Simulate a write failure (read-only repo file) → toast and revert.
- **Automation:** Covered by: `Triggers.test.tsx` (toggle) | Layer: unit | Gap: failure toast untested.
- **Needs relay:** no
- **Status:** Shipped

### TR-16 — Test fire

- **What:** **Test…** opens "Test “<name>”?", which warns "This really fires: <target> runs now…", lets the user edit the "Test payload" JSON (default `{"test": true}`; invalid JSON shows "Not valid JSON." and disables the button), and fires on **Fire it now**. Works while disabled and bypasses the storm guard and rate limit. Toast "Test fire queued a run" or the fire's status/error.
- **Where:** List ⋯ → Test…, trigger page header; `TriggerDialogs.tsx` (`TestTriggerDialog`); command `test_fire_trigger`; HTTP `POST /api/triggers/{id}/test`.
- **Preconditions:** A trigger (any type).
- **Steps:** 1. Disable a trigger. 2. Test with a custom payload. 3. Enter invalid JSON.
- **Expected:** A run starts with that payload; fire row status `queued`; invalid JSON shows a hint and disables Fire.
- **Edge / negative cases:** Testing a trigger with a missing target toasts the error.
- **Automation:** Covered by: `Triggers.test.tsx` | Layer: unit / api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-17 — Storm guard

- **What:** More than 5 fires within 10 seconds auto-disables the trigger, records the fire as `error` ("Fired more than 5 times in 10s, so it was disabled") and emits `trigger://error`. Cron and test fires are exempt.
- **Where:** `src-tauri/src/services/triggers/dispatch.rs` (`StormDetector`), `schema.rs` (`STORM_MAX_FIRES`, `STORM_WINDOW_MS`).
- **Preconditions:** A webhook trigger with no rate limit.
- **Steps:** POST to the webhook 7 times within 10 s.
- **Expected:** 6th fire is an error; trigger `enabled: false` in its file; later calls 404.
- **Edge / negative cases:** Re-enabling resets the guard.
- **Automation:** Covered by: `a_trigger_storm_disables_the_trigger`, `storms_are_more_than_five_fires_in_ten_seconds` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-18 — Rate limit

- **What:** If `rate_limit_seconds` hasn't passed since the last dispatched fire, the fire is recorded `rate_limited` ("Rate limited until <time>") and `trigger://rate_limited {nextAllowedAt}` is emitted; no run starts.
- **Where:** `dispatch.rs` (`rate_limited_until`).
- **Preconditions:** A trigger with `rate_limit_seconds: 3600`.
- **Steps:** Fire twice quickly.
- **Expected:** First queued, second `rate_limited` in the Firings tab.
- **Edge / negative cases:** Test fires ignore the limit.
- **Automation:** Covered by: `rate_limits_record_the_fire_without_dispatching`, `rate_limits_relative_to_the_last_dispatched_fire` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-19 — Trigger page: Settings tab

- **What:** Header with source icon, name, "<when> → Asks <agent>" or "<when> → Runs <workflow>", enabled switch, **Test…**, ⋯ **Delete…**. Settings tab: endpoint panel (webhook/device), "Source" select (switchable), the form, **Revert** and **Save changes** (disabled when unchanged or invalid). Leaving with unsaved changes asks first. Changing to webhook/device returns a new secret to show once.
- **Where:** Automations → Triggers → <trigger>; `src/components/views/TriggerDetailView.tsx` (`SettingsForm`); not-found state "Back to Automations".
- **Preconditions:** A trigger.
- **Steps:** 1. Change the name; click elsewhere in the sidebar. 2. Click **Save changes**. 3. Switch Source Schedule → Webhook and save.
- **Expected:** Leave guard dialog; toast "Trigger saved" and YAML updated; the new webhook secret is displayed.
- **Edge / negative cases:** Unknown trigger id shows "Trigger not found" ("No trigger with id “<id>”. It may have been deleted.") with Back to Automations.
- **Automation:** Covered by: `Triggers.test.tsx` (TriggerDetailView tests) | Layer: unit | Gap: leave guard on triggers not asserted.
- **Needs relay:** no
- **Status:** Shipped

### TR-20 — Firings tab (history)

- **What:** Every fire (columns Fired, Result, Request) with relative time (full time in a tooltip), status badge with its error: once the fire started a run, the run's status (Queued, Running, Waiting for approval, Completed, Failed, Cancelled; for a workflow also Waiting on a person, Paused), otherwise the fire's own (Rate limited, Error, Skipped), and a link "Open <target>" to the run it started; selecting a row shows "What it received" (the payload).
- **Where:** Trigger page → Firings; `TriggerDetailView.tsx` (`FiringsTable`, `FiringRow`), `trigger-ui.tsx` (`FireStatusBadge`); command `list_trigger_fires` (default 50); HTTP `GET /api/triggers/{id}/fires?limit=`.
- **Preconditions:** A trigger that has fired.
- **Steps:** Open Firings, select a row, click the link.
- **Expected:** Payload JSON shown; link opens the agent Run Detail or the workflow run view.
- **Edge / negative cases:** No fires: "This trigger hasn't fired yet.". Fires without a run show "—".
- **Automation:** Covered by: `Triggers.test.tsx` ("lists firings with their results and what each received"), `trigger-ui.test.tsx`, `triggers/tests.rs` (`a_webhook_queues_a_run_with_its_source_and_payload`: the fire follows its run to Completed) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-21 — Delete a trigger

- **What:** ⋯ → Delete… asks first ("Delete “<name>”?", button **Delete trigger**), then removes the file, its projection and its keyring secret, and stops its cron job/watcher.
- **Where:** `TriggerDialogs.tsx` (`DeleteTriggerDialog`); command `delete_trigger`; HTTP `DELETE /api/triggers/{id}`.
- **Preconditions:** A trigger.
- **Steps:** Delete from the list; then from the trigger page.
- **Expected:** File gone from `.nandi/triggers/`; row disappears; from the page, navigation returns to the list.
- **Edge / negative cases:** Workflow runs it started later show "a removed trigger".
- **Automation:** Covered by: `triggers/tests.rs` (indirect) | Layer: api | Gap: no explicit delete test.
- **Needs relay:** no
- **Status:** Shipped

### TR-22 — Trigger secrets in the OS keyring

- **What:** Webhook/device secrets live in the keyring under `nandi.trigger.<id>` (`secretRef`), never in the file or SQLite; listings expose only `hasSecret`; secrets are never logged. A trigger file pulled onto a new machine gets a fresh secret.
- **Where:** `src-tauri/src/services/triggers/mod.rs` (`attach_secret`), `src-tauri/src/services/secrets.rs`.
- **Preconditions:** OS keyring available.
- **Steps:** 1. Create a webhook trigger. 2. Inspect the YAML, `state.db` `triggers` rows, and the keyring.
- **Expected:** Secret only in the keyring.
- **Edge / negative cases:** Keyring unavailable (Linux without a secret service) → creation fails with an error.
- **Automation:** Covered by: `services/triggers/tests.rs` `secrets_live_in_the_keyring_not_in_sqlite_or_files` | Layer: api | Gap: real OS keyring is manual.
- **Needs relay:** no
- **Status:** Shipped

### TR-23 — Trigger audit log

- **What:** Every fire (including rate-limited, skipped, errors) appends metadata (trigger id, fire id, source/target type, target id, status, run id, error; never the payload) to the triggers audit log `~/.nandi/local/logs/triggers.log`.
- **Where:** `dispatch.rs` (`audit_fire`), `src-tauri/src/services/audit` (`AuditLog::Triggers`); visible in Settings › Logs, **Triggers** tab (logs area).
- **Preconditions:** None.
- **Steps:** Fire a trigger; read the log.
- **Expected:** One JSON line per fire without payload content.
- **Edge / negative cases:** None.
- **Automation:** Covered by: none specific | Layer: api | Gap: no test that payloads are excluded.
- **Needs relay:** no
- **Status:** Shipped

### TR-24 — Trigger events and live UI

- **What:** `trigger://changed`, `trigger://fired`, `trigger://rate_limited`, `trigger://error` refresh `['triggers']`, `['trigger', id]`, `['triggerFires', id]` so lists and history update live.
- **Where:** `src-tauri/src/services/triggers/mod.rs`, `src/hooks/useTriggers.ts`, `src/lib/agent-sync.ts`.
- **Preconditions:** None.
- **Steps:** Keep the Firings tab open and call the webhook from a terminal.
- **Expected:** The new fire appears without reload.
- **Edge / negative cases:** None.
- **Automation:** Covered by: `count(recorder, ...)` assertions in `triggers/tests.rs` | Layer: api | Gap: UI live refresh not tested.
- **Needs relay:** no
- **Status:** Shipped

### TR-25 — Trigger management HTTP API

- **What:** `GET/POST /api/triggers`, `POST /api/triggers/cron-preview`, `POST /api/triggers/calendar-preview`, `GET/PUT/DELETE /api/triggers/{id}`, `POST /api/triggers/{id}/enable|disable|secret|test`, `GET /api/triggers/{id}/fires`, plus receivers (TR-07, TR-10). Management routes need the web token; receivers use the trigger secret.
- **Where:** `src-tauri/src/http/triggers.rs`.
- **Preconditions:** Web token.
- **Steps:** Create, update, toggle, test, list fires and delete a trigger over HTTP.
- **Expected:** Same results as the UI; created responses include the secret once.
- **Edge / negative cases:** Management route without the token → 401.
- **Automation:** Covered by: `src-tauri/src/http/mod.rs` webhook test | Layer: api | Gap: CRUD routes lack HTTP tests.
- **Needs relay:** no
- **Status:** Shipped

### TR-26 — Trigger firings as Work items

- **What:** Agent runs started by trigger firings (last 24 h, up to 100 fires) appear as Work items of kind `trigger` in their run's state; a workflow firing appears as its workflow run.
- **Where:** `src-tauri/src/services/work/mod.rs` (`trigger_items`, `from_trigger_fire`); shown on Home Running/Recent.
- **Preconditions:** An agent-target trigger that fired.
- **Steps:** Fire it; call `GET /api/work`.
- **Expected:** An item with kind `trigger`, the trigger name as title, state matching the agent run.
- **Edge / negative cases:** Fires without a run id are skipped.
- **Automation:** Covered by: `a_trigger_firing_is_a_work_item_in_its_runs_state` (`work/tests.rs`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### TR-27 — Calendar source: times, days and who is responsible

- **What:** A `calendar` trigger fires at set local times (`times`) on set weekdays (`days`), except on `skipDates`. Who is responsible each day is decided in this order: a stand-in (`cover`) spanning the date, then a fixed weekday owner (`byDay`), then whose turn it is (`people` taking turns from `startDate`, `rotate: week | day`; daily turns skip days off). If the calendar names people and nobody is responsible on a day, it doesn't fire that day. The payload is `{scheduled_at, trigger_id, duty_id, date, role: primary, responsible, backup, covering_for, duty_reason}`, so templates can use `{{payload.responsible}}`. Default priority 80. It is exempt from the storm guard.
- **Where:** `src-tauri/src/services/domain/calendar.rs`, `src-tauri/src/services/triggers/calendar.rs`; New Trigger → **Calendar** tab (`src/components/triggers/CalendarFields.tsx`); command `preview_calendar`, `POST /api/triggers/calendar-preview`.
- **Preconditions:** A target agent.
- **Steps:** 1. Create a Calendar trigger: 09:00, Mon–Fri, people `alice, bob`, each turn a day, starting last Monday, plus a stand-in for next week. 2. Read **Next runs**. 3. Clear the start date.
- **Expected:** Next runs lists the dates with the responsible person, "(for …)" on stand-in days, and "backup: …". Without a start date, Create is disabled.
- **Edge / negative cases:** These are all rejected: `9am` as a time, `funday` as a day, a stand-in that ends before it starts, and an unknown field. A time that a daylight-saving change skips is skipped.
- **Automation:** Covered by: `domain::calendar` tests, `triggers::calendar` tests, `trigger-form.test.ts` ("calendar triggers"), `Triggers.test.tsx` ("shows who is on duty next, with their backup") | Layer: unit | Gap: no test that a real calendar job fires at its time.
- **Needs relay:** no
- **Status:** Shipped

### TR-28 — Calendar backup takes over

- **What:** Each day has a backup: the stand-in's `backup`, else the calendar's `backup`, else the next person in the turns (never the primary). The backup takes over when the primary's run fails, never starts, or isn't finished `backupAfterMinutes` after it started. Taking over fires the trigger again with `role: backup`, `responsible` set to the backup, plus `primary` and `handover_reason`. Rate limits don't apply to that fire.
  - A Nandi **not** named in the calendar (an owner, such as a team lead's) fires every day and watches its own run.
  - A Nandi **named** in the calendar runs only on its owner's days.
  - On its backup days it asks the relay to watch the duty (`DutyWatch`, relay design step 14). Five minutes after the time the relay asks the primary's Nandi (`DutyQuery` → `DutyReport`, from the primary's fire row and run status; a failure's error sealed for the backup) and decides: settled, ask again later, or hand over. The relay keeps the watch on disk, so restarting the relay loses nothing.
  - The relay hands over when the primary's Nandi is offline, doesn't answer within 15 s, or reports that the run failed, ran late or never started.
  - With the relay down past the 5 minutes, the backup's Nandi hands over by itself (the owner's decision 6).
  - A Nandi answers only about people named in that trigger's calendar.
- **Where:** `src-tauri/src/services/triggers/handover.rs` (`watch_remotely`, `RelayDutyNetwork`), `relay/client.rs` (`duty_watch`, `duty_report`, `on_duty`); nandi-relay `src/handover.rs`, `src/store/duties.rs`, `GET /team/watches`.
- **Preconditions:** For the cross-machine case: two instances on one relay (RL-11) whose display names are in the calendar, the same trigger file in both repos, and a relay built from this version.
- **Steps:** 1. Owner case: target an agent that fails, and set a backup. 2. Cross-machine case: on the primary's day, quit the primary's Nandi. 3. Same, with `backupAfterMinutes: 1` and a slow run.
- **Expected:** Each calendar firing says who it was for: "On duty: Dev Chechi", or "Lena Fischer took over from Dev Chechi: <reason>". 1. A second fire with `role: backup` appears in Firings. 2. About 5 minutes after the time, the backup's Nandi fires, with a `handover_reason` saying the primary is offline. 3. The backup fires once the minute has passed. If the primary's run completes, there is no second fire.
- **Edge / negative cases:** A third person named in the calendar does nothing that day. A question from someone not named in the trigger is ignored and logged.
- **Automation:** Covered by: `decides_when_the_backup_takes_over`, `reads_the_duty_from_the_fire_and_its_run` (`handover.rs`), `calendar_duties_are_tracked_and_handovers_skip_the_rate_limit` (`triggers/tests.rs`); nandi-relay `tests/handover.rs` (settled, failed with its error, overran, no answer, a watch across a relay restart) | Layer: api | Gap: the backup Nandi's side of a watch lives in memory, so a decision that arrives after the backup restarted is logged, not acted on.
- **Needs relay:** yes
- **Status:** Shipped

## Request page

### RQ-01 — Request page header and states

- **What:** One page per Request (gateway/delegation task chain or local ask), opened by any of its task ids: back button, title, Watch in companion (while running), Save as workflow and Do this again (local), Stop (running/waiting), a line with direction ("From <member> · Incoming" / "To <member> · Outgoing" / "On this Nandi"), start time, elapsed time (ticks live), status, "step n of m". Loading skeleton; "This request no longer exists" with "Back to the dashboard"; a load error shows "Couldn't load this request" with "Try again".
- **Where:** Links from Home items, Inbox, Run Detail "Part of", delegation thread, gateway rows, Ask; view kind `request`; `src/components/views/RequestDetailView.tsx`; command `get_request`; HTTP `GET /api/requests/{id}`.
- **Preconditions:** A delegated task or local ask.
- **Steps:** Open a Request; open a bogus id.
- **Expected:** Header renders with display names, not device ids; bogus id shows "This request no longer exists".
- **Edge / negative cases:** Any task id in a failover chain resolves to the same Request.
- **Automation:** Covered by: `RequestDetailView.test.tsx` ("shows the header…", "says when a request is gone"), `any_task_id_resolves_to_its_request` (`src-tauri/src/services/requests/tests.rs`) | Layer: unit / api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RQ-02 — Steps tab (system steps, plan steps, attempts)

- **What:** A step rail: incoming gateway tasks show system steps (Received, Allowed, Routed, Delivered) from `task_events`, then the workflow/agent steps; outgoing failover attempts stack under the peer step with why each moved on; the requester view collapses provider internals into "Working (step n of m)". Steps show durations (ticking while running) and can be focused via `stepId`.
- **Where:** Request → Steps; `src/components/requests/RequestSteps.tsx`, `request-model.ts` (`stepNumbers`, `stepDuration`, `focusStep`), `src-tauri/src/services/requests/mod.rs` (`project`).
- **Preconditions:** An incoming gateway task routed to a workflow; an outgoing task that failed over.
- **Steps:** Open each Request's Steps tab.
- **Expected:** System steps then workflow steps; both attempts listed with the first one's reason.
- **Edge / negative cases:** Plan steps are numbered; system steps aren't.
- **Automation:** Covered by: `an_outgoing_failover_shows_every_attempt_and_why_it_moved_on`, `an_incoming_gateway_task_routed_to_a_workflow_shows_its_steps` (`requests/tests.rs`), `RequestDetailView.test.tsx` ("stacks failover attempts under the peer step") | Layer: unit | Gap: two-instance run is manual.
- **Needs relay:** no
- **Status:** Shipped

### RQ-03 — Stuck banner with blocker and fix buttons

- **What:** When a Request is stuck, one banner explains why (blocker kinds: needs my/their approval, needs my/their answer, not trusted, not accepting, paused or blocked, queued, offline, no model, runtime auth, timed out, failed, declined), who must act, how long it's waited, queue position ("Place in line: n"), expiry, "+n more waiting", and offers the fix: Approve / Reject, Answer, "Trust <member>" / "Turn off requests from <member>", "Ask <member> to trust you" (copies a message), "Turn on for <agent>", Resume, Send elsewhere, "Try next provider", Cancel, Add a model, "Add an API key", "View details", "Ask <member>" (remind). Reject, Trust, Turn off and Cancel ask first. Tone: warning when it waits on me, neutral on others, red once ended. Blocker derivation prefers typed `reason_kind` over text.
- **Where:** Request page; `src/components/requests/RequestBanner.tsx`, `request-model.ts` (`bannerActions`, `needsConfirmation`, `bannerTone`), `src-tauri/src/services/requests/blocker.rs`.
- **Preconditions:** Requests in each blocked state (see `request-fixtures.ts`).
- **Steps:** For each blocker, open the Request and use the primary fix.
- **Expected:** e.g. not_trusted incoming shows "Trust <member>" (confirm) and "Turn off requests from <member>"; outgoing shows "Ask <member> to trust you"; no-model opens Settings › Models & runtimes; timed-out outgoing offers "Try next provider".
- **Edge / negative cases:** A typed refusal beats its text; a kind on a task that moved on is no blocker.
- **Automation:** Covered by: `blocker.rs` table tests (every kind), `request-model.test.ts`, `RequestDetailView.test.tsx` (approve, reject confirm, trust, ask-trust, models, retry) | Layer: unit | Gap: Remind/Send elsewhere flows untested end to end.
- **Needs relay:** no
- **Status:** Shipped

### RQ-04 — Outputs, Activity and Details tabs

- **What:** Outputs lists results/artifacts; Activity is a time-ordered event list with expandable payloads; Details lists capability, direction, routing decision, requester, provider, task ids (copyable), ETA, usage, and the raw reason (raw ids appear only here).
- **Where:** Request → Outputs / Activity / Details; `RequestDetailView.tsx` (`Outputs`, `Activity`, `Details`, `CopyValue`).
- **Preconditions:** A finished Request.
- **Steps:** Open each tab; copy a task id.
- **Expected:** Toast "Copied"; raw ids only in Details.
- **Edge / negative cases:** No outputs: empty message.
- **Automation:** Covered by: `RequestDetailView.test.tsx` ("lists activity and keeps raw IDs in Details") | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RQ-05 — Result first and rating

- **What:** A done Request shows its result block at the top; an outgoing delegation can be rated 1–5 stars once ("How did <member> do?") (stored as requester evidence).
- **Where:** Request page; `RequestDetailView.tsx` (`ResultBlock`); command `rate_work`; HTTP `POST /api/work/{id}/rating`.
- **Preconditions:** A completed outgoing delegation.
- **Steps:** Click 4 stars.
- **Expected:** the stars show the rating; `capability_evidence.user_rating = 4`.
- **Edge / negative cases:** Rating outside 1–5 → "Ratings go from 1 to 5"; unfinished task → "Only finished delegated tasks can be rated".
- **Automation:** Covered by: `RequestDetailView.test.tsx` ("puts the result first when done and rates an outgoing request"), `finished_delegations_can_be_rated` | Layer: unit / api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RQ-06 — Stop a Request

- **What:** Stop asks first ("Stop this request?": "<member> is told to stop." for delegations, "The work in progress stops." for local asks; "Keep it" backs out), then cancels the task (`cancel_task`) or the local Request and everything its running steps started (agent turns, workflow runs, tasks).
- **Where:** Request header → Stop; `RequestDetailView.tsx`; commands `cancel_task`, `cancel_request`; HTTP `POST /api/requests/{id}/cancel`, `POST /api/tasks/{id}/cancel`.
- **Preconditions:** A running Request.
- **Steps:** Stop → Keep it; Stop → confirm.
- **Expected:** Toast "Stopped"; status cancelled; linked runs cancelled.
- **Edge / negative cases:** Stopping an ended local Request → "This request has already ended".
- **Automation:** Covered by: `RequestDetailView.test.tsx` ("asks before stopping and cancels the task", "stops the Request itself") | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RQ-07 — Retry a failed outgoing request (attempt n+1)

- **What:** "Try next provider" (banner) or Retry (Inbox) creates a new attempt linked through `previous_attempt_id` (next provider) instead of an unlinked delegation; the page shows both attempts.
- **Where:** Banner "Try next provider"; Inbox failed row Retry; command `retry_task`; HTTP `POST /api/requests/{id}/retry`; `requests/mod.rs` (`retry_task`).
- **Preconditions:** A timed-out or failed outgoing delegation.
- **Steps:** Click "Try next provider" on the Request page.
- **Expected:** New attempt appears under the peer step.
- **Edge / negative cases:** Retrying an incoming task is not offered.
- **Automation:** Covered by: `RequestDetailView.test.tsx` ("tries the next provider for a timed-out outgoing request") | Layer: unit | Gap: backend retry linking not directly asserted.
- **Needs relay:** no
- **Status:** Shipped

### RQ-08 — "Part of" a Request from a run

- **Duplicate:** tested under RN-12. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### RQ-09 — Live Request updates

- **What:** The page refreshes on `task://state_changed`, `task://progress`, `work://item_updated` and `request://updated`.
- **Where:** `src/hooks/useRequests.ts`, `request-model.test.ts` ("refreshes on the events that change a Request").
- **Preconditions:** A running Request.
- **Steps:** Keep the page open while the task progresses.
- **Expected:** Steps and status update without reload.
- **Edge / negative cases:** None.
- **Automation:** Covered by: `request-model.test.ts` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## Local asks as Requests

### RQ-10 — An action ask becomes a Request

- **What:** An approved plan from Ask (or a plan that only points at people/manual steps, which starts without asking) is saved as a Request (`requests`, `request_steps`) and carried out by the backend; the app navigates to its page with toast "Started. Follow it on its page.".
- **Where:** Ask box anywhere (the Ask page, Home, workflow run view); `src/components/ask/AskBox.tsx` (`startPlan`, `planNeedsApproval`); command `start_request`; HTTP `POST /api/requests`; `src-tauri/src/services/requests/local.rs` (`start_request`).
- **Preconditions:** A model; agents/workflows to act on.
- **Steps:** 1. Ask "have <agent> summarize X then run workflow Y". 2. Approve the plan.
- **Expected:** Request page opens with linked steps; `request://updated` events.
- **Edge / negative cases:** Empty ask → "There's nothing to do: the ask is empty"; a rejected or empty plan → "This plan has no steps to carry out".
- **Automation:** Covered by: `an_ask_becomes_a_request_on_the_same_page` (`local.rs`) | Layer: api | Gap: plan-approval rule in AskBox not unit-tested here (Ask area).
- **Needs relay:** no
- **Status:** Shipped

### RQ-11 — Local Request execution, linking and restart resume

- **What:** Steps start in order (each after the ones it depends on), each links the agent run, workflow run or task it started (following compacted child runs), a failed step fails the Request and skips the rest, and open Requests resume on restart (`resume_all`). One runner per Request, polling every second.
- **Where:** `local.rs` (`tick`, `start`, `observe`, `settle`, `spawn_runner`, `resume_all`).
- **Preconditions:** A multi-step plan.
- **Steps:** 1. Start a 2-step Request. 2. Quit during step 1; relaunch.
- **Expected:** Step 2 starts only after step 1 is done; after relaunch the Request continues.
- **Edge / negative cases:** Agent run waiting on approval shows the step as Waiting.
- **Automation:** Covered by: `a_new_request_runs_its_steps_in_order`, `the_next_step_starts_when_the_one_before_is_done`, `settles_from_its_steps`, `a_failed_step_fails_it_and_skips_the_rest`, `stores_and_reloads_a_request`, `a_request_picks_up_after_a_restart` | Layer: unit / api | Gap: real restart manual.
- **Needs relay:** no
- **Status:** Shipped

### RQ-12 — Mark a waiting step done (local Request)

- **What:** Steps of kind "ask a person" or "manual" wait for the user; the Steps tab offers **Mark done**, which completes it and lets the Request continue.
- **Where:** Request → Steps; command `complete_request_step`; HTTP `POST /api/requests/{id}/steps/{position}/complete`.
- **Preconditions:** A local Request with a human/manual step.
- **Steps:** Click **Mark done** on the waiting step.
- **Expected:** Step Done; next step starts.
- **Edge / negative cases:** Completing a step not waiting → "Only a step waiting on you can be marked done".
- **Automation:** Covered by: `RequestDetailView.test.tsx` ("marks a step waiting on you done") | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RQ-13 — Do this again

- **What:** For an ended local Request, **Do this again** re-runs the same plan as a new Request and opens it.
- **Where:** Request header; command `rerun_request`; HTTP `POST /api/requests/{id}/rerun`.
- **Preconditions:** An ended local Request.
- **Steps:** Click Do this again.
- **Expected:** New Request id, same steps, page navigates to it.
- **Edge / negative cases:** Not shown for gateway/delegation Requests or while running.
- **Automation:** Covered by: `RequestDetailView.test.tsx` ("does it again as a new Request") | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RQ-14 — Save a Request as a workflow

- **What:** **Save as workflow** drafts YAML from the Request's plan (agent steps carry the label and ask text; other steps become manual steps), shows it for editing, and saves through `createWorkflow` (domain validation).
- **Where:** Request header; `src/components/requests/SaveAsWorkflowDialog.tsx`; command `draft_workflow_from_request`; HTTP `POST /api/requests/{id}/workflow-draft`; `local.rs` (`workflow_yaml`).
- **Preconditions:** A local Request.
- **Steps:** Click Save as workflow; edit the name; Save.
- **Expected:** File in `.nandi/workflows/`; the workflow validates and runs.
- **Edge / negative cases:** Invalid edits show the validation error in the dialog.
- **Automation:** Covered by: `drafts_a_workflow_that_the_domain_accepts`, `RequestDetailView.test.tsx` ("saves it as a workflow after you confirm the draft") | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## Work items and Inbox

### RQ-15 — Work items API

- **What:** `list_work` returns workflow runs in flight or from the last 24 h, agent runs started by trigger firings, outgoing top-level delegated tasks, and local Requests (items their steps started are folded into them), newest first, optionally filtered by state (active, waiting, done, failed, cancelled). Each item has kind, title, state, owner, progress, next action (with `reason_kind` and target ids), rating.
- **Where:** commands `list_work`, `get_work_item` (`src-tauri/src/commands/work.rs`); HTTP `GET /api/work`, `GET /api/work/{id}`; `src-tauri/src/services/work/mod.rs`; consumed by Home Running/Recent and the Inbox; `work://item_added|updated|removed` events.
- **Preconditions:** Some recent work.
- **Steps:** Call `GET /api/work` and `GET /api/work?state=waiting`.
- **Expected:** Items as described; gateway sub-tasks hidden under their parent.
- **Edge / negative cases:** Unknown id → "No work item '<id>'".
- **Automation:** Covered by: `work_lists_active_waiting_and_recent_done` (`work/tests.rs`) | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Partial (`next_action.description` is still an English sentence with raw step/agent ids, `work/mod.rs` `from_run`)

### RQ-16 — Inbox on Home

- **What:** A section at the top of Home listing everything waiting on the user, one calm line each (what it is and, when it has one, how long it has left): missing model, tool approvals, workflow step approvals, held gateway requests, work assigned to the user, escalated clarification questions, people asking to be trusted, manual workflow steps, work from mail, the calendar or a teammate waiting for the user's OK (the intake queue, one line per thread), then failures from the last day. Ordered by soonest deadline, then age; failures last. Re-evaluated every 30 s and on events. Clicking a line opens the item's Ask chat (RQ-34), where its card with the fix sits on top; the companion keeps the cards inline (AI-13).
- **Where:** Home (Sidebar → Home); `src/components/inbox/Inbox.tsx` (`InboxRow`, `InboxItemCard`), `inbox-model.ts` (`buildInbox`, `inboxRowLabel`, `inboxItemView`), `src/hooks/useInbox.ts`, `src/components/home/HomeView.tsx`. A request to a person is titled with what was asked (not its capability, `assign-me`) and shows "To {name}", the person's name rather than their instance ID (`services/work/mod.rs`, `task_title`, `member_names`).
- **Preconditions:** Items of several kinds pending.
- **Steps:** Open Home; compare order with deadlines.
- **Expected:** One line per item, soonest expiry first, with no buttons in the list; "Nothing needs you right now." when empty.
- **Edge / negative cases:** Items past their deadline drop out; a run waiting on a manual step shows once (as the step); news for queued work shows once (its thread).
- **Automation:** Covered by: `src/components/inbox/inbox-model.test.ts` (the intake queue, thread IDs), `src/components/home/Home.test.tsx` ("Home, Inbox": one line, opening its chat) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RQ-17 — Inbox count badge

- **What:** The Home nav item shows the number of Inbox items waiting on the user (failures not counted); the title bar/companion use the same count.
- **Where:** Sidebar → Home badge; `src/components/layout/Sidebar.tsx`, `nav-tree.ts`, `inbox-model.ts` (`inboxBadgeCount`).
- **Preconditions:** Pending items.
- **Steps:** Create one approval and one failure.
- **Expected:** Badge shows 1.
- **Edge / negative cases:** Zero hides the badge.
- **Automation:** Covered by: `inbox-model.test.ts` ("counts what waits on you, not failures") | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RQ-18 — Tool approvals in the Inbox, with expiry

- **What:** Pending tool approvals from every agent (`list_pending_approvals`) render as the shared approval card with a plain-language summary ("Run a command: git push origin main"; a request to someone on the network names them and quotes it: "Ask Lena Fischer · PR Reviewer (review-pr): “…”"), JSON collapsed under Details, the asking agent's name, "Expires in N min", Approve and Reject (Reject asks first). Approvals expire after 1 hour (`APPROVAL_TTL_MS`, swept every minute; expired = reject). Deciding anywhere clears it everywhere.
- **Where:** Home → Inbox; `src/components/agent-console/ApprovalRequestCard.tsx`, `src/components/inbox/approval-model.ts`; command `list_pending_approvals` / `approve_action`; HTTP `GET /api/approvals/pending`, `POST /api/agents/{id}/approve`; `src-tauri/src/services/chat.rs` (`expire_overdue_approvals`), `approvals.expires_at`.
- **Preconditions:** An agent with `requires_approval: [shell]` and a model.
- **Steps:** 1. Make it run a shell command. 2. Approve from the Inbox. 3. Repeat and wait over an hour.
- **Expected:** Run resumes on approve; after an hour it disappears and the run shows the approval as expired.
- **Edge / negative cases:** Deciding in Run Detail removes it from the Inbox.
- **Automation:** Covered by: `approval-model.test.ts`, `inbox-model.test.ts` ("drops approvals past their deadline") | Layer: unit / manual (real LLM) | Gap: sweep timing manual.
- **Needs relay:** no
- **Status:** Shipped

### RQ-19 — Workflow step approvals in the Inbox

- **What:** A workflow run waiting for a step approval shows "<workflow> is waiting for your approval" with "Step: <step>" and Approve/Reject (Reject asks "Reject step <step>?"), or Review to open it.
- **Where:** Home → Inbox; `Inbox.tsx` (`WorkflowApprovalRow`).
- **Preconditions:** A run awaiting approval.
- **Steps:** Approve from the Inbox.
- **Expected:** Row disappears; run continues.
- **Edge / negative cases:** The row shows the raw step id, not the step label used on the workflow page.
- **Automation:** Covered by: `inbox-model.test.ts` | Layer: unit | Gap: row actions untested.
- **Needs relay:** no
- **Status:** Shipped

### RQ-20 — Held gateway requests ("hold, don't refuse")

- **What:** A capability on "Needs my approval" no longer refuses another Nandi's request: the task stays Delivered at stage `approval`, a `gateway_holds` row puts "<member> asked for <capability>" with their words and the time left in the owner's Inbox with Approve / Decline (Decline asks "Decline <member>'s request?"). Approve continues routing; Decline rejects with `declined`; no decision within 1 hour (or the task's own deadline) expires as no. The requester sees "Waiting for <member> to approve this request". Holds survive restarts.
- **Where:** Home → Inbox → the hold's chat (RQ-34); `Inbox.tsx` (`GatewayHoldRow`); commands `list_gateway_holds`, `decide_gateway_hold` (`taskId, approve, note?`: a refusal's reason or an approval's condition, sent to the requester); HTTP `GET /api/gateway/holds`, `POST /api/gateway/holds/{taskId}/decide` (`{approve, note?}`); `src-tauri/src/services/gateway/holds.rs` (`HoldWaiters::set_note`), `router.rs` (`hold_for_approval`, `declined_reason`); event `gateway://hold_changed`.
- **Preconditions:** Two instances on a relay; a capability set to Needs my approval; caller trusted.
- **Steps:** 1. From B, request the capability on A. 2. On A, Approve from the Inbox. 3. Repeat and Decline. 4. Repeat and wait an hour.
- **Expected:** (2) toast "<member>'s request goes on", completes; (3) B sees "<member> declined this request" with Send elsewhere; (4) expires.
- **Edge / negative cases:** Requester cancels while held → hold closes as rejected. Declined from the chat with a reason → B sees "The owner declined this request: <reason>"; approved with a condition → B's thread notes "Approved by the owner: <condition>". The note is kept in memory until routing takes it: a restart between the decision and the requester's answer loses it (the decision stands).
- **Automation:** Covered by: `a_capability_needing_approval_waits_for_the_owner`, `a_refusal_from_the_holds_chat_tells_the_requester_why` (`src-tauri/src/services/a2a/tests.rs`), `holds.rs` tests, `inbox-model.test.ts` (held requests) | Layer: api / manual (two machines) | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RQ-21 — Busy gateway queues requests

- **What:** When the gateway has no free slot, an incoming request waits in line instead of being refused; the banner says "Waiting in line at <member>" (requester) or "Waiting in line: your Nandi is busy" (provider) with "Place in line: n", and Details shows the raw reason "Waiting for a free slot"; it times out after 10 minutes (or the requester's deadline).
- **Where:** `src-tauri/src/services/gateway/router.rs` (`wait_for_slot`, `QUEUE_WAIT_MS`); Request banner (queued blocker, position).
- **Preconditions:** Gateway concurrency 1; two requests at once.
- **Steps:** Send two requests from B quickly.
- **Expected:** Second shows position 1 then completes after the first.
- **Edge / negative cases:** Still waiting after 10 min → timed out "This Nandi was still busy when time ran out".
- **Automation:** Covered by: `a_busy_gateway_queues_the_request_until_a_slot_frees` (`a2a/tests.rs`) | Layer: api | Gap: none.
- **Needs relay:** yes
- **Status:** Shipped

### RQ-22 — Typed reasons on task events

- **What:** Task transitions and refusals carry `reason_kind` (matching blocker kinds) and `stage` (admit, policy, approval, slot, route, dispatch, child, result) in `task_events`, and `queue_position` on progress; the Request blocker uses them before falling back to text.
- **Where:** `src-tauri/src/services/a2a/task.rs`, `task_state.rs`, `executor.rs`, `router.rs`; `requests/blocker.rs` (`from_reason_kind`).
- **Preconditions:** Delegation traffic.
- **Steps:** Trigger a trust refusal and a queue; read `task_events`.
- **Expected:** Rows with `reason_kind` `not_trusted` / `queued` and stage values.
- **Edge / negative cases:** Old rows without kinds still map via text.
- **Automation:** Covered by: `blocker.rs` (`every_reason_kind_maps_to_its_blocker`, `a_typed_refusal_beats_its_text`, `a_held_or_queued_task_waits_without_ending`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RQ-23 — Other Inbox items: clarifications, missing model, trust requests, manual steps

- **What:** Escalated clarification questions (with an approval id, not expired) show "A question for you: <question>" with **Answer** (opens the delegation thread); a missing default model shows "No model connected, so agents can't run." with "Connect a model" (Settings › Models & runtimes); people refused for lack of trust show "<name> wants to send you work" with "Trust <name>" / "Not now"; manual steps show the ManualStepCard (WF-26).
- **Where:** `Inbox.tsx` (`SimpleRow`, `TrustRequestRow`, `ManualStepCard`).
- **Preconditions:** Each situation.
- **Steps:** Use each row's action.
- **Expected:** Each opens/does its fix and the row leaves.
- **Edge / negative cases:** Questions the agent answers itself never appear.
- **Automation:** Covered by: `inbox-model.test.ts` ("shows only questions escalated to a person…", trust requests) | Layer: unit | Gap: row UI untested.
- **Needs relay:** no
- **Status:** Shipped

### RQ-24 — Failed items: Retry and Dismiss

- **What:** Failures from the last 24 h appear after waiting items (not counted in the badge) with **Dismiss** (hidden from the Inbox, kept in history; remembered in `work_preferences`) and **Retry** (a workflow run asks first, "Run <title> again?", since every step runs again; a delegation retries as a new attempt). The title opens the item.
- **Where:** `Inbox.tsx` (`FailedRow`); commands `dismiss_work`, `list_dismissed_work`, `run_workflow`, `retry_task`; HTTP `POST /api/work/{id}/dismiss`, `GET /api/work/dismissed`.
- **Preconditions:** A failed workflow run and a failed delegation.
- **Steps:** Dismiss one; Retry the other (confirm for workflow).
- **Expected:** Dismissed row gone and stays gone after restart; retry starts new work.
- **Edge / negative cases:** Dismissing an empty id → "Nothing to dismiss"; dismissals older than 7 days are pruned.
- **Automation:** Covered by: `dismissed_failures_are_remembered` (`work/tests.rs`), `inbox-model.test.ts` ("offers failures from the last day that were not dismissed") | Layer: unit / api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RQ-28 — Told when a request comes back

- **What:** A request you sent that comes back after the live wait (an answer, Done, Decline, an error, or no answer by its due date) is recorded (`request_returns`), shown in the Inbox as "Lena answered" with the answer and what you asked, and sent as `request://returned`. The companion turns it into an OS notification while hidden, unless Settings › Appearance › **Notify me when requests come back** is off (`companion.notifyResults`, on by default). **Dismiss** takes it out of the Inbox. A request that came back failed shows once, as the return, not also as a generic failure. Returns older than 7 days drop off.
- **Where:** `services/a2a/returns.rs` (`from_result`, `insert`, `list`, `dismiss`), `history.rs` (`close`), `db.rs` (`request_returns`), commands `list_request_returns`, `dismiss_request_return`, `set_companion_notify_results`; HTTP `GET /api/delegation/returns`, `POST /api/delegation/returns/{id}/dismiss`, `POST /api/companion/notify-results`. Frontend: `inbox-model.ts` (`requestReturn`), `Inbox.tsx` (`RequestReturnRow`), `useInbox.ts` (`useRequestReturns`), `companion-model.ts` (`noticesFor`, `results`), `useCompanionNotifications.ts`, `AppearancePane.tsx`.
- **Preconditions:** Two Nandis on a relay. The receiver doesn't answer within the live wait (for example, offline).
- **Steps:**
  1. Ask the other person a question; their Nandi is off.
  2. Hide the companion. Start their Nandi and let their Chief of Staff answer.
  3. Turn the setting off and repeat.
- **Expected:**
  - Step 2: a notification "<name> answered" with the answer; the Inbox lists it; **Dismiss** removes it.
  - Step 3: the Inbox item appears, but no notification.
- **Edge / negative cases:** an answer that arrives during the live wait shows in the chat and isn't recorded as a return.
- **Automation:** Covered by: `returns.rs` tests, `a2a/tests.rs` (`an_answer_that_comes_after_the_live_wait_is_kept`), `inbox-model.test.ts` ("requests that came back"), `companion-model.test.ts` ("notifies about a request that came back") | Layer: unit | Gap: a request reassigned after the live wait comes back under its new attempt's ID, which has no history row, so it isn't shown as a return yet.
- **Needs relay:** yes
- **Status:** Shipped

### RQ-25 — Agent run queue (one turn per agent)

- **Duplicate:** tested under CH-11. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### RQ-26 — Watch a Request in the companion

- **Duplicate:** tested under AI-15. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### RQ-29 — Chief of Staff acts on what arrives and shows Since you were away

- **What:** Work that arrives from outside (email, calendar) is screened for phishing and malware signs and waits in a queue for your approval. **Approve** starts a Request, or adds the news to work already running instead of starting a second Request. Chats start work directly. Home shows a "Waiting for your OK" section at the top of the card (screening verdict, reasons, sender, **Approve** and **Dismiss**), and the card shows a "Since you were away" card (titled "What your Chief of Staff did" before the first "Got it") with a summary line and one row per item the Chief of Staff acted on: needs you, started, updated (with the news) or finished (done, failed, cancelled), the source (email, calendar, chat) and the time. A row opens its Request. **Got it** marks the digest seen and clears every row except the needs-you ones.
- **Where:** Home (`ChiefOfStaffDigest.tsx`, `digest-model.ts`, `useChiefOfStaffDigest.ts`), commands `get_chief_of_staff_digest`, `mark_chief_of_staff_digest_seen`, `approve_queued_work`, `dismiss_queued_work`, event `request://updated`; HTTP `GET /api/chief-of-staff/digest`, `POST /api/chief-of-staff/digest/seen`, `POST /api/chief-of-staff/queue/{id}/approve`, `POST /api/chief-of-staff/queue/{id}/dismiss`.
- **Preconditions:** A mailbox connected (Settings › Email), a model that works, a second account to send mail from.
- **Steps:**
  1. Send yourself a mail from another account: "Your power bill of $84 is due on Friday".
  2. Wait for the email check, or press **Check now** in Settings › Email.
  3. Open Home.
  4. Send a follow-up: "the due date moved to Monday". Check again.
  5. Click **Approve** on the queued mail.
  6. Click **Approve** on the follow-up's news.
  7. Click **Got it**.
  8. Send a phishing-style mail: "Your password expires today, sign in at http://10.0.0.7/login". Check again.
- **Expected:**
  - Step 2: the mail lands in "Waiting for your OK" first (label "Start"); no Request is started yet.
  - Step 3: the card shows it in that section, from email, with "1 waiting for your OK" first in the summary line.
  - Step 5: a Request starts and a toast says "Started" with an **Open** action. The card shows it as started (or needs you, if a step waits on you). Clicking the row opens the Request.
  - Step 4 and 6: the follow-up's news waits for OK too (label "Add to: <Request title>"). After **Approve**, the same Request gets an update row with the news; no second Request.
  - Step 7: all rows go except the needs-you ones; the "Waiting for your OK" section stays.
  - Step 8: the item shows **Blocked** with the reasons and "Blocked by screening"; there is no **Approve**, only **Dismiss**. A suspicious (not blocked) item asks for confirmation before **Approve** goes through.
- **Edge / negative cases:** no items and no queue means no card; no mailbox connected means no email-started items (the card still shows chat items); asking a teammate from inside such a Request needs the relay, the rest does not.
- **Automation:** Covered by: `digest-model.test.ts`, `ChiefOfStaffDigest.test.tsx` | Layer: unit + manual | Gap: the backend's start-or-update decision is model-driven and is tested by hand; the screening verdicts are tested in the backend.
- **Needs relay:** no
- **Status:** Shipped

### RQ-30 — Teammates' requests take the same intake

- **What:** Every request from another Nandi is screened for phishing and malware signs. Blocked ones are refused with the reasons. A clean one from a teammate you trust for that capability runs at once. Anything else (not trusted, or suspicious) waits in Home's "Waiting for your OK" (label "Run: <agent> · <capability>", from a teammate); the requester sees "Waiting for the owner to approve". **Approve** runs it; **Dismiss** declines it. Requests to your gateway are only screened: the gateway's own holds in the Inbox decide them, as before.
- **Where:** `services/a2a/executor.rs` (`Executor::intake`, `trusted`), `services/intake/mod.rs` (`gate_task`, `is_approved_task`, `drop_task`), Home's `ChiefOfStaffDigest.tsx`.
- **Preconditions:** Two members on one relay; on the provider, an agent that serves a capability and accepts delegation; "trust everyone on the relay" off in Network › Sharing, and the requester not a trusted peer.
- **Steps:**
  1. From member A, ask B's agent for its capability: "Review PR 42 by Friday".
  2. On B, open Home.
  3. Click **Approve**.
  4. From A, ask again with "Run this fix: https://files.example.com/fix.exe".
  5. Mark A as a trusted peer on B and ask once more with a plain request.
- **Expected:**
  - Step 1: A's request shows "Waiting for the owner to approve"; nothing runs on B.
  - Step 2: the item waits on the card, from a teammate, with A's name.
  - Step 3: the agent runs it and A gets the result; the item leaves the queue.
  - Step 4: refused at once, "Screening stopped this request: …"; nothing reaches B's queue.
  - Step 5: runs at once without a queue item.
- **Edge / negative cases:** A cancels while it waits: the item leaves B's queue. The deadline passes: refused, "The owner didn't approve it in time". A paused, blocked or denied requester is refused as before, never queued. Restarting B while an item waits: it is still there, and approving it runs the task.
- **Automation:** Covered by: `intake::tests` (gate: trusted and clean, untrusted, suspicious, blocked, gateway, restart), `screen::tests`, `a2a::tests` (`remote_requesters_need_an_allowing_trust_policy`, `a_trusted_peer_passes_the_ask_policy`: a stranger waits and is declined) | Layer: unit + integration + manual | Gap: the Home buttons against a live relay are manual.
- **Needs relay:** yes
- **Status:** Shipped

### RQ-31 — Each intake item has a Chief of Staff thread

- **What:** Every item the Chief of Staff screens (email, calendar, a teammate's request) has its own conversation with it, a thread (`intake:<id>`). News for the same work joins the same thread. The backend writes the Chief of Staff's notes into it (queued, approved, dismissed). Home's rows open the thread. The thread page shows the item with its screening verdict and plan, **Approve** and **Dismiss**, the Requests it became (with their steps and **Mark done**), and the conversation. Asking for a draft gets a draft; asking to give the work to an agent queues an item in the same thread for approval instead of running it. **Mark done** on Home's "Needs you" row finishes a "For you" step.
- **Where:** `IntakeThreadView.tsx`, `intake-thread-model.ts`, Home's `ChiefOfStaffDigest.tsx`, `services/intake/thread.rs`, `get_intake_thread`, `send_intake_message`.
- **Preconditions:** Email connected (or a calendar event) so an item reaches the queue; an agent that can do the work.
- **Steps:**
  1. Let an email arrive that asks for something. Open Home and click the item.
  2. Read the thread.
  3. Ask: "draft a reply".
  4. Ask: "give this to <agent>".
  5. Click **Approve** in the thread header.
  6. For a plan with a "For you" step, go back to Home and click **Mark done** on its "Needs you" row.
- **Expected:**
  - Step 1: the thread opens with the item's title and source.
  - Step 2: the Chief of Staff's note says it queued the item; the header shows the screening verdict and the plan steps.
  - Step 3: the Chief of Staff answers with a draft.
  - Step 4: a new item waits in the same thread; nothing runs yet.
  - Step 5: it starts, the thread gets an "approved" note, and the Request appears below with its steps.
  - Step 6: the step finishes and the row leaves "Needs you".
- **Edge / negative cases:** A blocked item shows "Blocked by screening" and only Dismiss. A suspicious item asks before it starts. Dismissing adds a "dismissed" note and the item moves to the history line. A teammate's request opens its thread too, and its Request has no Mark done. Without a relay everything works except teammates' items, which need the relay. The queue is an Inbox kind too: its line opens this thread. Saying "approve it" (or "dismiss it") in the thread makes the Chief of Staff propose `inbox_approve` / `inbox_refuse`, confirmed on its card like any Inbox chat decision (RQ-35).
- **Automation:** Covered by: `digest-model.test.ts`, `intake-thread-model.test.ts`, `inbox-model.test.ts` (the intake queue), `services::intake::thread` tests | Layer: unit + manual | Gap: the page against live email is manual.
- **Needs relay:** no
- **Status:** Shipped

### RQ-32 — Auto-approve rules

- **What:** Rules let some screened items through without a click. Settings › Auto-approve lists the rules (a domain such as `chase.com`, an email address, or a teammate), each with its actions as checkboxes (mail: start work, add news; teammate: run their requests), and **Add rule**. Screening still runs first: only a clean item passes, a suspicious one waits, a blocked one is refused. A mail rule only matches mail whose sender passed the sender check (DMARC), and a domain covers its subdomains. An item a rule approves is noted in its thread ("Approved automatically (mail from chase.com)"), shows in "Since you were away", and is in the decisions file as decided by the rule (`auto-approve rule (mail from chase.com)`, PO-14). On a queued clean email item, **Always allow mail from <domain>** adds a domain rule with the item's action, after a confirm. The form explains a missing action, a bad address or domain and a full list (100) before saving; the core's answer shows as a toast. Teammate rules need the relay; mail rules don't.
- **Where:** `AutoApproveSection.tsx`, `auto-approve-model.ts`, `useAutoApproveRules.ts`, `QueueRowView.tsx` (`AlwaysAllowDialog`), `services/intake/rules.rs`, `dmarc_verified` in `services/email/mail.rs`, `get_auto_approve_rules`, `set_auto_approve_rules`, `GET`/`PUT /api/chief-of-staff/auto-approve`.
- **Preconditions:** Email connected, and a test sender whose mail passes DMARC (a second domain to send from as well). Teammate rules: a relay with a teammate.
- **Steps:**
  1. Open Settings › Auto-approve. Read the explanation and the empty state.
  2. Pick **Email domain**, type the test sender's domain, keep both actions and click **Add rule**.
  3. Let a verified mail that asks for something arrive from that domain.
  4. Let a mail that asks for something arrive from another domain.
  5. Let a suspicious mail (a link to a download, "run this file") arrive from the rule's domain.
  6. Open Home and the thread of the item from step 3.
  7. On the item from step 4, click **Always allow mail from <domain>** and confirm.
  8. Remove both rules (the trash button).
- **Expected:**
  - Step 1: the page says screening comes first and a forged sender never passes; with no rules it suggests "your bank's domain, e.g. chase.com".
  - Step 2: the rule shows "Mail from <domain>" with both actions ticked. Typing `chase` or `https://chase.com` shows an error and **Add rule** stays off; clearing both actions shows "Pick at least one action".
  - Step 3: the work starts at once, with no queue item. Its thread says "Approved automatically (mail from <domain>)" and Home's "Since you were away" lists it.
  - Step 4: it waits in "Waiting for your OK".
  - Step 5: it still waits, with its warnings; nothing starts.
  - Step 7: the confirm says it only applies to verified mail; after **Always allow** a toast says mail from the domain is approved automatically and the rule is in Settings.
  - Step 8: the rules go; the next mail from either domain waits again.
- **Edge / negative cases:** A mail whose sender failed DMARC (a forged `From:`) never matches an email rule, even from the rule's domain. A subdomain (`alerts.chase.com`) matches a `chase.com` rule. A blocked item is refused whatever the rules say. The teammate option without a relay shows "Connect to your relay", and the other two kinds work. A teammate's queued item has no **Always allow** button (the queue doesn't carry their instance ID): add the rule in Settings.
- **Automation:** Covered by: `auto-approve-model.test.ts`, `digest-model.test.ts`, `intake::rules` tests, `email::tests` (DMARC), `approvals_suite.rs` `auto_approvals_are_recorded_as_such` (a teammate rule set over the API lets a clean request through, recorded as decided by `auto-approve rule (requests from …)`; the groups' approval as `group policies`; a group that asks first holds it whatever the rule says, and the person's approval is recorded as theirs) | Layer: unit + api + manual | Gap: the Settings form and the shortcut against live mail are manual.
- **Needs relay:** no
- **Status:** Shipped

### RQ-33 — Chief of Staff does a step of its own plans, and offers Save as workflow

- **What:** A work plan can give Chief of Staff one step it does itself (a site in its browser, its notes, mail triage). The step runs in the item's thread when the work came through intake, else in the Request's own conversation, never in Chief of Staff's main chat; the step finishes when that turn ends and the plan goes on. Work Chief of Staff starts from that turn waits in **Waiting for your OK**. A plan never holds two Chief of Staff steps. When work finishes well and the same work (same title, ignoring dates and numbers) or the same multi-step plan finished well before, Chief of Staff notes a proposal in the thread, and **Save as workflow** shows on the Request card in the thread and on Home's finished row. It opens the workflow draft, where the run plan's hand-overs (sign in, pay) are manual steps; nothing is saved until **Save workflow**.
- **Where:** `services/requests/self_step.rs`, `services/requests/recurrence.rs`, `local::workflow_yaml`, `IntakeThreadView.tsx`, `ChiefOfStaffDigest.tsx`, `SaveAsWorkflowDialog.tsx`, the playbooks `delegate-work` and `build-a-workflow`.
- **Preconditions:** Email connected and a site Chief of Staff may browse (Browser pilot running). For the proposal: the same work done once before (for example last month's bill).
- **Steps:**
  1. Let an email ask for something Chief of Staff can do on a website (pay a bill on a test site). Open its thread from Home and read the plan.
  2. Click **Approve**.
  3. Watch the thread while the step runs; open Chief of Staff's main chat.
  4. In the thread, ask Chief of Staff to start more work for itself.
  5. Repeat steps 1 and 2 with the same kind of email (next month's bill) and let it finish.
  6. Click **Save as workflow** on the Request card in the thread.
  7. Open Home's **Since you were away**.
- **Expected:**
  - Step 1: the plan names Chief of Staff for the website step.
  - Step 2: the step starts in this thread: its run plan card shows, sign-in and payment are handed over to you.
  - Step 3: the main chat has nothing new; when the turn ends, the step shows done and the Request moves on or finishes.
  - Step 4: a new item waits in **Waiting for your OK**; nothing starts.
  - Step 5: the thread gets a note offering to save it as a workflow, naming the earlier work.
  - Step 6: the draft opens; the sign-in and payment are manual steps, the rest are Chief of Staff steps. Cancel saves nothing; **Save workflow** saves and opens it.
  - Step 7: the finished row has **Save as workflow** too.
- **Edge / negative cases:** Work the user asks for in the ask box with a Chief of Staff step runs in the Request's own conversation. A plan with two Chief of Staff steps keeps the first and warns. Failed work, work that is only manual steps, and work that already ran a workflow are never proposed. Without a relay everything works.
- **Automation:** Covered by: `services::requests::self_step`, `services::requests::recurrence` and `services::requests::local` tests (the step runs in the thread and settles, the loop guard queues, the draft cuts at hand-overs, the proposal shows), `ask::tests::the_planner_may_give_the_chief_of_staff_one_step`, `digest-model.test.ts`, `intake-thread-model.test.ts`, `ChiefOfStaffDigest.test.tsx` | Layer: unit + integration + manual | Gap: a live browser step is manual.
- **Needs relay:** no
- **Status:** Shipped

### RQ-34 — Every Inbox item opens as an Ask chat

- **What:** Opening an Inbox line (Home, the Inbox page or the work tray) opens that item's chat with the Chief of Staff: a conversation `inbox:<kind>:<id>` made the first time it's opened (an intake item keeps its `intake:<id>` thread, RQ-31). The item's card sits on top with its buttons as shortcuts, the conversation below, and the ask box in chat mode at the bottom. The Chief of Staff gets the item and what's being decided as untrusted `<task-data>`, so the user can ask "why?", "what did they send?", "what happens if I refuse?", or answer in their own words. Once the item no longer waits, the top says "Nothing is left to decide here" and the chat stays.
- **Where:** `src/components/views/InboxItemThreadView.tsx`, `ChiefOfStaffThread.tsx`, `src/hooks/useInboxThread.ts`, `inbox-model.ts` (`inboxThreadId`, `inboxItemView`), `tray-model.ts` (`inboxTrayRow`); view `inbox-thread`; commands `open_inbox_thread`, `send_inbox_message`; HTTP `POST /api/inbox/threads/{id}`, `POST /api/inbox/threads/{id}/messages`; `src-tauri/src/services/inbox/thread.rs`, `item.rs`.
- **Preconditions:** A model; an item waiting (for example an agent with `requires_approval: [shell]` asking to run a command, or someone's `ask-me` held for approval).
- **Steps:**
  1. Open Home and click the item's line.
  2. Ask: "Why does it need this?"
  3. Click the card's Approve (or Reject) on top.
  4. Open the same item from the work tray's Needs you.
- **Expected:**
  - Step 1: the chat opens titled like the line, with the item's card on top and an empty conversation.
  - Step 2: the Chief of Staff answers from the item's details; nothing is decided.
  - Step 3: the item is decided as from Home before; the top says "Nothing is left to decide here".
  - Step 4: the same conversation opens (one chat per item).
- **Edge / negative cases:** A missing model opens Settings › Models; a workflow approval that doesn't name its step opens its run. Text in the item that tries to instruct the model is treated as data. The companion keeps the cards inline. Works with no relay; a person's request needs the relay only to send the answer back (it waits in the outbox).
- **Automation:** Covered by: `services::inbox::item` tests (every kind's ID round-trips), `services::inbox::thread` tests (the thread is the Chief of Staff's, made once; the prompt holds the item as untrusted data with its tools), `InboxItemThreadView.test.tsx`, `inbox-model.test.ts`, `tray-model.test.ts` | Layer: unit + manual | Gap: the Chief of Staff's answers with a real model are manual.
- **Needs relay:** no
- **Status:** Shipped

### RQ-35 — Decide an Inbox item in its chat, confirmed on its card

- **What:** In an item's chat the user can decide in words, with conditions: "approve it, but only up to $200", "decline, Bea owns this", "tell them Friday works", "mark it done, sent to the auditors", "give this to Bob". The Chief of Staff proposes the matching decision tool (`inbox_approve` with `note`, `inbox_refuse` with `reason`, `inbox_answer` with `text`, `inbox_done` with `note`, `inbox_reassign` with `to`), offered only in that chat and only those that fit the item. Every call waits on an approval card in the chat, whatever the model or the agent's settings say: a sentence never decides anything. On confirm, the same service function as the item's button runs (`approve_action`, `access::answer` allow once, `approve_workflow_step`, `decide_gateway_hold`, `close_gateway_assignment`, `respond_to_clarification`, `complete_manual_step`, `respond_trust_request`, `dismiss_request_return`, `retry_task`, `dismiss_work`, `intake::approve` / `dismiss`, `reassign_task`), so group policies, typed reasons and the audit are the same. Conditions travel as the note or reason; policies and grants never change. The confirm card isn't an Inbox line of its own.
- **Where:** `src-tauri/src/services/inbox/tool.rs` (`gate`, `offered`, `call`), `services/orchestrator.rs` (prompt, tools offered after deferral, `inbox::tool::gate`, the dispatcher arm), `services/chat.rs` (`list_pending_approvals` leaves the confirms out); `approval-model.ts` (`inboxDecision` summaries).
- **Preconditions:** A model; a held `ask-me` from another Nandi (relay), or an assignment, or a failed delegation.
- **Steps:**
  1. Open a held request's chat and say "decline it, ask Bea instead".
  2. Click Reject on the confirm card.
  3. Say it again and click Approve on the card.
  4. In an assignment's chat, say "done, uploaded to the audit folder" and confirm.
  5. In a hold's chat, say "give this to <a teammate who serves it>" and confirm.
- **Expected:**
  - Step 1: a card "Refuse this item, because: ask Bea instead" waits; the hold is still pending.
  - Step 2: nothing is decided; the Chief of Staff is told it was refused.
  - Step 3: the hold is declined and the requester sees "The owner declined this request: ask Bea instead".
  - Step 4: the assignment closes Done and the requester gets the note as the result.
  - Step 5: the task moves to the teammate (NW-12) with the reason.
- **Edge / negative cases:** A tool that doesn't fit the item (an answer to a hold) is refused with the tools that do. An item decided elsewhere first: the call says it no longer waits and changes nothing. A browser hand-over can only be skipped from a chat; a connector access request is allowed once only (lasting grants stay on the card). A failed workflow run is retried from its button only (it may send things again). A teammate who isn't online, or two who match, is named back. A decision tool named outside an Inbox chat is refused.
- **Automation:** Covered by: `services::inbox::tool` tests (`every_decision_call_waits_for_the_user`, `a_chat_decision_waits_for_the_users_confirm_then_calls_the_service` through the orchestrator, `a_refused_confirm_decides_nothing`, `a_decision_that_does_not_fit_the_item_changes_nothing`, `done_on_an_assignment_sends_the_note_back_and_is_recorded`, `a_returned_request_is_dismissed_from_its_chat`, `reassigning_finds_the_teammate_by_name_or_id`), `a_refusal_from_the_holds_chat_tells_the_requester_why` (`a2a/tests.rs`) | Layer: unit + integration + manual | Gap: a real model choosing the tool is manual.
- **Needs relay:** no
- **Status:** Shipped

### RQ-36 — Every Inbox decision is kept in the gold copy

- **What:** Every decision on an Inbox item, from a button or a chat, is appended to `.nandi/decisions/<yyyy-mm>.md` in the repo (staged in git, backed up like any definition): when (UTC), who decided (this Nandi's user), the item's kind and ID, what it was about, what was decided, the reason or note, and the chat's conversation ID when it came from a chat. The chat's text stays in `state.db`. Auto-approve rules record their decisions too, named in the reason.
- **Where:** `src-tauri/src/services/inbox/audit.rs` (`record`, `in_chat`), `services/repos/decision_repo.rs`, `services/domain/decision.rs`, `repos/layout.rs` (`decisions_dir`); one `record` call in each decision service (`chat::approve_action`, `workflows::approve_workflow_step`, `gateway::decide_gateway_hold`, `close_gateway_assignment`, `manual_steps::complete_manual_step`, `clarification::respond_to_clarification`, `trust_requests::respond_trust_request`, `returns::dismiss_request_return`, `requests::retry_task`, `work::dismiss_work`, `intake::approve_as` / `dismiss`, `reassign::reassign_task`).
- **Preconditions:** Any Inbox item.
- **Steps:**
  1. Decline a held request from its card with no chat.
  2. Approve another from its chat with a condition.
  3. Open `~/.nandi/repo/.nandi/decisions/<this month>.md` and run `git status` in the repo.
- **Expected:** Two entries, oldest first: the first with "Decision: declined" and no Chat line; the second with "Decision: approved", "Reason: <the condition>" and "Chat: `inbox:gatewayHold:<id>`". The file is staged. No chat text is in it.
- **Edge / negative cases:** The confirm on a chat's card isn't a second entry. A note can't forge a heading or a field (later lines are indented). A file that can't be written is logged; the decision still stands. A new month starts a new file with its heading.
- **Automation:** Covered by: `domain::decision` tests, `repos::decision_repo` tests, `services::inbox::audit` tests, `the_button_and_the_chat_record_the_same_decision` (`inbox::tool` tests) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RQ-37 — Decisions page: past Inbox decisions, month by month

- **What:** Home › Decisions lists every decision kept in `.nandi/decisions/<yyyy-mm>.md` (RQ-36), newest first, one calm row each: a badge for what was decided (Approved, Refused, Declined, Answered, Done, …, green for yes, red for no), the item's label, the time, its kind, "by {name}", and the reason. A month picker (newest month first) switches files; the search box (item, id, reason), Kind and Decision filters narrow the month. A decision made in a chat shows "From a chat": a link to that chat while the chat is still on this computer, otherwise just the chat's id. With no decision yet the page says every decision from a button or an Ask chat is recorded here and kept in your repo. The Inbox has a "Past decisions" link to the page. Purely local: it reads the repo's files and works with no relay. A hand-edited file is read as far as it makes sense: an entry that lost its time, item or decision is skipped (and logged), the others still show.
- **Where:** Sidebar → Home → Decisions; Inbox header → "Past decisions" (`src/components/inbox/Inbox.tsx`); `src/components/views/DecisionsView.tsx`, `src/components/decisions/decisions-model.ts`, `src/hooks/useDecisions.ts`; Tauri commands `list_decision_months`, `list_decisions` (`commands/decisions.rs`); HTTP `GET /api/decisions/months`, `GET /api/decisions?month=&kind=&decision=&search=` (`http/decisions.rs`); `services/inbox/decisions.rs`, `repos/decision_repo.rs` (`months`, `entries`), `domain/decision.rs` (`parse_month`, `DecisionFilter`, `select`).
- **Preconditions:** Some Inbox decisions in this and an earlier month (RQ-36); one of them made in a chat.
- **Steps:**
  1. Open Home → Decisions (also from the Inbox's "Past decisions").
  2. Check the order, then pick the earlier month.
  3. Filter by Kind, then Decision, then type part of a reason in the search box; clear the filters.
  4. Click "From a chat" on the chat decision.
  5. In a fresh `NANDI_HOME`, open the page.
- **Expected:** 1-2. Newest decision first; the picker lists months newest first and the earlier month shows its own entries. 3. Only matching rows stay; "No decisions match" with Clear filters when none do. A decision like "approved (once)" is found by Approved. 4. Opens the chat in the Inbox-thread view. 5. The empty state with the explanation; no month picker.
- **Edge / negative cases:** A chat that no longer exists shows its id, not a link. Decisions of a month without a file aren't listed. A file with a broken entry still lists the others. A decision text the page doesn't know shows as written. Works offline (no relay). Reading is read-only: nothing is written.
- **Automation:** Covered by: `domain::decision` tests (round trip, hand edits, filters), `repos::decision_repo` tests (months, entries), `services::inbox::decisions` tests, `offline_suite` ("decisions/months" answers with no relay), `DecisionsView.test.tsx`, `decisions-model.test.ts`, `nav-tree.test.ts` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### RQ-38 — Getting to know you: know-me to-dos and proposals

- **What:** After onboarding, the Inbox shows one quiet to-do (kind `knowMe`, not counted in the badge): "Tell me about your work, and anything at home you'd like help with". It opens the one know-me chat with the Chief of Staff (`inbox:knowMe:you`), which also opens any time from Network › Sharing › What you offer › **Tell Nandi about you** (with the latest confirmed changes listed). The Chief of Staff asks a few follow-ups (its `know-me` playbook) and proposes changes with `knowme_propose`, offered only there and always waiting on a card in the chat. On the card the person edits each change (text; **On my card** for work; Private or one of their groups for personal; remove notes, offers, suggestions) and confirms or refuses. On confirm: work items on the card join NANDI.md `responsibilities:`, off-card work and notes go to My notes `about-me.md`, personal items to `private/about-me.md` only (never NANDI.md), a personal item shared with a group also to `private/groups/<group>.md`; description and "What I can / can't help with" edit NANDI.md; offered capabilities leave `hide`; suggestions are shown only. The decision is recorded (`knowMe`, "Getting to know you", personal answers only counted) and the to-do closes. **Later** brings it back in a week, **Skip** in a month; a new to-do comes at most once a month, and three months after an answer it asks "Is what Nandi knows about you still right?". A member of the chosen group who asks this Nandi gets that group's shared items in the Chief of Staff's answer; nobody else does.
- **Where:** Inbox (Home), the work tray, Network › Sharing › What you offer; `src/components/knowme/` (`KnowMeTodoCard`, `KnowMeProposalCard`, `TellNandiAboutYou`, `knowme-model.ts`), `src/hooks/useKnowMe.ts`, `inbox-model.ts` (`knowMe`), `ApprovalRequestCard.tsx`; commands `get_know_me`, `know_me_todo`, `confirm_know_me_proposal`; HTTP `GET /api/know-me`, `POST /api/know-me/todo`, `POST /api/know-me/proposals/{approval_id}/confirm`; event `knowme://changed`; `src-tauri/src/services/knowme/` (`cadence.rs`, `proposal.rs`, `sharing.rs`), `services/inbox/item.rs` (`KnowMe`), `tool.rs` (`knowme_propose`, `confirmed_args`), `thread.rs`; playbook `resources/builtin-agents/chief-of-staff/skills/know-me.md`; `docs/developer/know-me.md`.
- **Preconditions:** A model; onboarding finished (or Settings › Run setup again to Done). For group sharing, a relay and a group with a second Nandi.
- **Steps:**
  1. Finish onboarding and open Home.
  2. Open the to-do and say: "I'm a scrum master for two teams on Jira, and I have two kids at school."
  3. On the card, switch one work item off the card and pick a group for a personal item (if you have one); click **Confirm**.
  4. Open Network › Sharing › What you offer and `~/.nandi/repo/NANDI.md`, My notes (`about-me.md`, `private/about-me.md`), and Home › Decisions.
  5. Click **Tell Nandi about you**, say something new, then **Refuse** the card.
  6. With a fresh home, finish onboarding, open the to-do and click **Skip**; then **Later** on another.
  7. From the second Nandi (a member of the group), ask the first: "What does Sam look after at home?"; from a Nandi outside the group, ask the same.
- **Expected:**
  - Step 1: the to-do is in the Inbox list, quiet, and the badge count doesn't change.
  - Step 2: the Chief of Staff asks at most two or three follow-ups, then a card "Getting to know you · changes to confirm" waits; nothing is written yet.
  - Step 3-4: NANDI.md lists only the work items left on the card; the personal ones are only in `private/about-me.md` (and the group's note); What you offer shows the new responsibilities; Decisions has "Getting to know you" with "on the card: …" and counts; the to-do is gone; Tell Nandi about you lists the change.
  - Step 5: the same chat opens; nothing is written.
  - Step 6: Skip and Later each close the to-do with a toast; it doesn't come back on reload.
  - Step 7: the group member's answer can use the shared item; the outsider's can't.
- **Edge / negative cases:** A personal item asked onto the card stays off it. A group the person isn't in is refused on the card ("isn't a group you're in") and the card keeps waiting. Offering a capability no agent serves is refused (suggest instead). A proposal that changes nothing is refused. Text with newlines can't forge a heading in a note. Onboarding again doesn't add another to-do. With no model, the to-do stays and the help guide ("know me") explains it. Works with no relay (only the group member's answer needs it).
- **Automation:** Covered by: `services::knowme::cadence` tests (first once, skip a month, later a week, at most one a month, recheck), `services::knowme::proposal` tests (personal never on the card, group only one I'm in, served-only offers, sections, lines added once), `services::knowme::tests` (`a_proposal_writes_nothing_until_confirmed_then_writes_where_each_part_belongs` through the orchestrator with an edited card, `a_personal_item_is_shared_only_with_a_group_i_am_in`, `only_a_member_of_the_group_gets_what_is_shared_with_it`, `the_first_to_do_comes_after_onboarding_and_skip_puts_it_away`), `inbox::thread` (`the_know_me_chat_always_takes_a_proposal`), `inbox::item` (round trip), `help` tests, `offline_suite` (`with_no_relay_getting_to_know_you_works`, `know-me` route), `KnowMeProposalCard.test.tsx`, `KnowMe.test.tsx`, `inbox-model.test.ts` | Layer: unit + integration + manual | Gap: a real model's questions and proposals, and the group member's answer over a real relay, are manual.
- **Needs relay:** no
- **Status:** Partial (the relay's people search can't find a group-shared item: needs per-group card fields in the protocol; "Is this still yours?" from playbook proposals isn't wired, only the three-month recheck)

## Gaps and planned (not testable yet)

- No way to delete a workflow in the UI or API; only by removing its YAML file (no `delete_workflow` command).
- Workflow runs started by triggers take no input: the trigger's message template is ignored for workflow targets (`docs/developer/triggers.md` "Workflows don't take input yet"; `dispatch.rs`).
- Workflow run rows and firings open the workflow-run project view (`work-item`), not a Request page; `automations-model.ts` `runRequestView` says this is "until task 14" (`docs/tasks-done/task-2026-10-18-automations.md`).
- Workflows run strictly in order; no parallel steps or branches (data edges are drawn but don't change execution).
- `WorkItem.next_action` should be `{reason_kind, target ids}` with frontend copy; it still includes an English `description` with raw ids (`docs/tasks-done/task-2026-10-18-hold-dont-refuse.md`, `services/work/mod.rs` `from_run`).
- Work items as a view over the `requests` table, with gateway and trigger work migrated onto it, is not done; work is still assembled from runs, fires, tasks and local requests (`docs/tasks-done/task-2026-10-19-local-asks-as-requests.md`).
- Workflow step approvals have no expiry and no "Expires in" (only tool approvals and gateway holds expire).
- Manual step deadlines are display-only: nothing escalates or fails when overdue.
- Device triggers have no device discovery or pairing (`docs/status-report-2026-10-06.md`).
- Docs drift: `docs/developer/triggers.md` describes "Definition / Fires / Payload" tabs (code: Settings / Firings) and a dashboard "Recent triggers" card; the user guide says "Work → Waiting on me" for manual steps, but there is no Work page: they are in Home's Inbox.
- Planned (tasks-todo): triggers shipped in plugin packs, installed off; policies applying to workflow steps (`task-x-enforced-policies.md`); a CloudWatch → SNS → webhook trigger template.

### RQ-27 — What was asked, word for word

- **What:** Every Request carries its ask as sent (`Request.input`: the root task's input, or a local ask's text). The Request page and the Run Detail Overview of a run that served one open with **"<name> asked" / "You asked <name>" / "You asked"**: the text exactly as written (selectable), then every other input (due date, links, parameters), the capability and the deadline. In chat, the requester's request card is labelled **Asked: "<first line>"** and its details show the words asked above the raw arguments; the provider's delegated conversation starts with the ask.
- **Where:** `services/requests/mod.rs` (`Request.input`), `requests/local.rs`, `components/requests/AskedBlock.tsx`, `request-model.ts` (`askOf`), `RequestDetailView.tsx`, `RunDetailView.tsx` (`RunAsk`), `agent-console/ToolCallCard.tsx`, `tool-label.ts`.
- **Preconditions:** Two members who trust each other, or a local ask.
- **Steps:** 1. Ask a teammate's agent something with a due date and a link. 2. Open the Request on both sides. 3. Open the provider's run.
- **Expected:** Both sides show the same words and inputs; the run's Overview shows who asked and what.
- **Edge / negative cases:** An input with no text shows only its fields; a run not serving a Request shows nothing extra.
- **Automation:** Covered by: `request-model.test.ts` (`askOf`) | Layer: unit + manual | Gap: the views are manual.
- **Needs relay:** no
- **Status:** Shipped
