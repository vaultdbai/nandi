# Workflows

Multi-agent pipelines: each step sends a message to an agent and waits for its reply before the next step starts. Code: `src-tauri/src/services/workflows.rs` (definition, engine, persistence) and `src/components/views/Workflow*View.tsx`.

## Definition

A workflow is a YAML file in the gold copy at `.nandi/workflows/{id}.yaml` (`~/.nandi/repo/.nandi/workflows/`), or agent-scoped at `.nandi/agents/<agent>/workflows/{id}.yaml`:

```yaml
id: daily-report # must match the file name
name: Daily Report
steps:
  - id: research
    agent: research-agent # folder in ~/.nandi/repo/.nandi/agents
    input: "Find today's AI news"
  - id: summarize
    agent: writer-agent
    input: 'Summarize: {{research.output}}'
    critical: true
  - id: publish
    agent: publisher-agent
    input: 'Send to #ai-news: {{summarize.output}}'
    requires_approval: true
```

- `{{step.output}}` is an **earlier** step's final reply (the agent run's last non-empty assistant message). References to later or unknown steps are rejected when the file is parsed.
- `requires_approval: true` pauses the run **before** the step's message is sent.
- `critical: true` queues the step at `PRIORITY_WORKFLOW_CRITICAL` (30) instead of `PRIORITY_WORKFLOW_NORMAL` (50), ahead of other background work for the same agent.
- Unknown keys are errors, so typos fail loudly.

The file is the source of truth. The `workflows` table mirrors it (for `getWorkflows` and the `workflow_runs` foreign key): on startup, and on every `getWorkflow` / `runWorkflow` / `createWorkflow`. Invalid files are skipped and logged. "+ New Workflow" in the sidebar imports a YAML file through `createWorkflow`. It won't overwrite a different workflow that has the same ID.

## Execution

`WorkflowEngine::run_workflow(id)` inserts a `workflow_runs` row and returns its ID. A driver task then advances the run:

```text
pending ─▶ requires_approval and not yet approved ─▶ awaiting_approval (driver stops)
        ─▶ chat::submit to the agent's orchestrator ─▶ running
running ─▶ agent://turn_done (completed) ─▶ done ─▶ next step … ─▶ workflow://done
        ─▶ agent://turn_done (failed/cancelled) ─▶ failed ─▶ workflow://failed
```

- **An ordinary agent run.** A step is submitted with source `workflow`, the workflow run's ID, and the label `<workflow name> · step <n>`. It's an agent run in the agent's conversation, so it shows up in the agent's chat, dashboard, and run history. The engine learns about `turn_done` by subscribing to the `BroadcastSink`. It subscribes _before_ reading the run's status from the DB, so a turn that ends in between is never missed. Lag, or 30s of silence, triggers a re-read.
- **Resumable.** All step state (status, rendered input, output, agent run ID) is stored in `workflow_runs.context` and re-read before each step. On startup, `resume_interrupted` restarts the driver for `running` runs. A step whose agent run was interrupted fails, because `fail_interrupted_runs` already marked that agent run as failed. Runs waiting for approval just wait.
- **One driver per run** (the `driving` set), so a double approval or a resume can't run a step twice.
- If the agent is busy, the step waits in its queue (see [Agent Runtime](./agent-runtime.md#one-turn-per-agent)). Removing it from the queue fails the step, and the run.

Approvals: `approveWorkflowStep(runId, stepId, decision)`. Approving resumes the driver. Rejecting fails the run without messaging the agent.

### Manual steps (wave EF)

A step with `step_type: manual` waits for a person instead of messaging an agent:

```yaml
- id: laptop
  step_type: manual
  instructions: 'Tag the laptop for {{accounts.output}} and paste the tag'
  deadline_hours: 48 # optional
```

Reaching it suspends the run (`waiting`, step `awaitingInput`) with no driver task; `completeManualStep(runId, stepId, { output, attachmentIds })` makes the reply the step's output and resumes the run. Because the state is in `workflow_runs`, a run can wait across restarts for days. Details: [Network and Reassignment](./network-and-reassignment.md#manual-steps-long-running-workflows).

### Ask steps (network v1)

A step with an `ask:` block sends one request to a colleague's Nandi and waits for the answer, which becomes `{{step.output}}`:

```yaml
- id: evidence
  ask:
    find: 'SOC 1 evidence' # or person: <instance id>, or capability: review-pr
    input: 'Please send the FR-07 approval log: {{scope.output}}'
    due: 5 days # optional; else 2 days for a question, 1 for anything else
```

- **Exactly one target** (`domain::workflow::AskStep`, checked in `Workflow::check`): `find` (the best match on the network for the topic, through their gateway, with the capability `find_people` would use), `person` (that instance's gateway, `ask-me`), or `capability` (any provider, ranked by the delegator). No `agent`, not manual.
- **Asks first when found by topic:** `start_ask` resolves the target (`resolve_ask`: `person` names from `network::known_members`; `find` asks the relay's people search, so without a connected relay the step fails with `RelayUnavailable`) and stores it on the step run (`askTarget`, `askCapability`, `asked`), then waits for approval with "Ask <name>: <text>" when the step used `find` or has `requires_approval`. Nothing is sent before that.
- **Sending:** one `DelegationRequest` with a preassigned task ID (`taskId` on the step run), in the background. The step goes to `awaitingInput` and the run to `waiting`, like a manual step; the step's `deadline` is the request's due date. A request that can't be sent is recorded in the delegation history as failed.
- **The answer:** `settle_asks` finds waiting runs whose ask task's `delegations` row has ended. Completed: the answer (`returns::text_of` of the output) is the step's output and the run goes on. Failed, declined or past its due date: the step fails (the run too, unless `on_failure: continue`). `watch_asks` runs it at startup (answers that came while the app was closed) and every 15 s.
- **Not yet:** cancelling a run doesn't withdraw a request already sent; it just stops waiting for it.

## Events

Emitted after the state is persisted. They aren't recorded in `run_events`, because the snapshot (`getWorkflow`) has everything:

| Event                               | When                                   |
| ----------------------------------- | -------------------------------------- |
| `workflow://step_started`           | A step's message was sent              |
| `workflow://step_awaiting_approval` | A step is paused for approval          |
| `workflow://step_done`              | A step's agent run completed           |
| `workflow://done`                   | Every step is done                     |
| `workflow://failed`                 | A step failed or was rejected          |
| `workflow://changed`                | Workflow files were added/changed/gone |

`agent-sync` invalidates the `['workflow', id]` query on run events, and `['workflows']` on `workflow://changed`.

## Workflows as capability providers (wave A)

A workflow can provide capabilities, the same way an agent does:

```yaml
id: python-review
name: Python Review
provides: [code-review.python] # optional; capability IDs
steps:
  - id: review
    agent: reviewer
    input: 'Review {{input}} for Python {{input.python_version}}'
  - id: summarize
    agent: writer
    input: 'Summarize: {{review.output}}'
```

- **Registration.** `capability::workflow_provider::refresh` keeps this instance's `capability_providers` rows with `provider_kind = 'workflow'` in step with every workflow's `provides:`. It runs at startup and whenever a workflow or agent changes, and it emits `provider://registered` and `provider://unregistered`. It never touches agent rows. A workflow's `provided_tools` are the tools of its step agents. Mismatches against the capability's schemas (no step reads `{{input}}`, a field the input schema doesn't declare, a non-string output schema) are logged as warnings and never fail the load.
- **Dispatch.** When the provider-side executor gets a task whose target is `<instance>/<workflow_id>` (or `<instance>/workflow:<workflow_id>`, which picks the workflow even when an agent has the same ID), and that workflow provides the task's capability, `a2a::executor::run_workflow` runs it. It applies the same pause, block, and trust checks an agent gets. For remote requesters, the trust policy is keyed by the workflow ID. The task goes to `Acknowledged`, then `Working`, and sends one progress update per settled step when the task asked for progress. It finishes within the task's deadline and ends like the workflow run. `{{input}}` in a step template is the task's input: its `text` field, a plain string, or JSON. `{{input.<field>}}` is one field of it. The result is the last step's output.
- **Evidence.** Each run records a `workflow_steps` task message with per-step status and duration. The aggregation job copies it into `capability_evidence.step_breakdown` and marks the row `provider_kind = 'workflow'`. `capability::aggregation` also caches 30-day aggregates in `capability_aggregates` every hour and clusters clarification questions into `capability_ambiguities`. `promote_ambiguity_to_schema` turns a cluster into an optional input-schema property.

## Manual test

1. Create two agents (e.g. `echo-a`, `echo-b`) in the Agent Console.
2. Save this as `~/.nandi/repo/.nandi/workflows/echo-chain.yaml` (the watcher picks it up), or import it with **+ New Workflow**:

   ```yaml
   id: echo-chain
   name: Echo Chain
   steps:
     - id: first
       agent: echo-a
       input: 'Reply with exactly one word: hello'
     - id: second
       agent: echo-b
       input: 'Repeat this back in capitals: {{first.output}}'
   ```

3. Open **Workflows → Echo Chain** and click **Run**. The transcript should show `first` finishing before `second` starts, and `second`'s input should contain `first`'s output. Each agent's chat should contain exactly the message its step sent.

To test without a model, set the `engine` key in the `local_settings` table of `~/.nandi/local/state.db` to `"canned"`. Agents then reply from the canned script.
