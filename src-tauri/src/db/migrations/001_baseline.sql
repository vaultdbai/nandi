-- Migration 001: the baseline. The whole state.db schema as it stood when
-- numbered migrations began (2026-11-15). Shipped migrations never change:
-- add the next numbered file instead (docs/developer/data-model.md).

CREATE TABLE conversations (
    id          TEXT PRIMARY KEY,
    agent_id    TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    title       TEXT
);
CREATE INDEX conversations_agent ON conversations(agent_id);

CREATE TABLE messages (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id          TEXT,
    role            TEXT NOT NULL,
    content         TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    metadata        TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX messages_conversation ON messages(conversation_id, created_at);
CREATE INDEX messages_run ON messages(run_id);

CREATE TABLE runs (
    id              TEXT PRIMARY KEY,
    agent_id        TEXT NOT NULL,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    started_at      INTEGER NOT NULL,
    ended_at        INTEGER,
    status          TEXT NOT NULL,
    input           TEXT NOT NULL,
    summary         TEXT,
    -- LLM usage summed over the run's model calls
    input_tokens    INTEGER NOT NULL DEFAULT 0,
    output_tokens   INTEGER NOT NULL DEFAULT 0,
    cost_usd        REAL,
    model           TEXT,
    -- Who asked for the run: user | workflow | agent | scheduled
    source_type     TEXT NOT NULL DEFAULT 'user',
    -- The workflow run or delegating agent
    source_id       TEXT,
    source_label    TEXT,
    -- Lower runs sooner
    priority        INTEGER NOT NULL DEFAULT 100,
    -- Set if the run had to wait; `started_at` is when it actually began
    queued_at       INTEGER,
    queue_position  INTEGER,
    -- The run this one continues after compaction, and its summary
    parent_run_id      TEXT,
    compaction_summary TEXT,
    -- The runtime that ran it: `native`, `claude-code`, ...
    runtime         TEXT
);
CREATE INDEX runs_agent ON runs(agent_id, started_at);
CREATE INDEX runs_status ON runs(status);

-- Runs waiting for their agent's current turn, with the input to start
-- them with. A row is deleted when its run starts or is cancelled.
CREATE TABLE agent_queue (
    id           TEXT PRIMARY KEY,
    agent_id     TEXT NOT NULL,
    run_id       TEXT NOT NULL,
    source_type  TEXT NOT NULL,
    source_id    TEXT,
    source_label TEXT,
    priority     INTEGER NOT NULL,
    message      TEXT NOT NULL,
    -- The user message's ID, created when the run starts
    message_id   TEXT NOT NULL,
    -- AttachmentInfo[] (JSON)
    attachments  TEXT,
    created_at   INTEGER NOT NULL
);
CREATE INDEX agent_queue_order ON agent_queue(agent_id, priority, created_at);

-- Projection of `triggers/<id>.yaml`: rules that start agent or workflow
-- runs when something happens. `source_config` never holds a secret (it's
-- in the keyring).
CREATE TABLE triggers (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    enabled            INTEGER NOT NULL DEFAULT 1,
    -- cron | file | webhook | device | event
    source_type        TEXT NOT NULL,
    -- Per source type (JSON)
    source_config      TEXT NOT NULL,
    -- agent | workflow
    target_type        TEXT NOT NULL,
    target_id          TEXT NOT NULL,
    -- {{payload.path}}, {{trigger.name}}, {{fired_at}}, {{last_run.output}}
    input_template     TEXT,
    priority           INTEGER NOT NULL DEFAULT 80,
    rate_limit_seconds INTEGER,
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    source_path        TEXT,
    projection_version INTEGER
);
CREATE INDEX triggers_enabled ON triggers(enabled, source_type);

-- Runtime state of each trigger, kept when the projection is rebuilt
CREATE TABLE trigger_state (
    trigger_id    TEXT PRIMARY KEY,
    last_fired_at INTEGER
);

-- Every time a trigger fired, whether or not it dispatched a run
CREATE TABLE trigger_fires (
    id         TEXT PRIMARY KEY,
    trigger_id TEXT NOT NULL,
    fired_at   INTEGER NOT NULL,
    -- The event that fired it (JSON)
    payload    TEXT NOT NULL,
    -- The agent run or workflow run it dispatched
    run_id     TEXT,
    -- queued | rate_limited | error | skipped
    status     TEXT NOT NULL,
    error      TEXT
);
CREATE INDEX trigger_fires_trigger ON trigger_fires(trigger_id, fired_at DESC);

CREATE TABLE run_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id      TEXT NOT NULL,
    kind        TEXT NOT NULL,
    payload     TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX run_events_run ON run_events(run_id);

CREATE TABLE approvals (
    id           TEXT PRIMARY KEY,
    agent_id     TEXT NOT NULL,
    run_id       TEXT NOT NULL,
    message_id   TEXT NOT NULL,
    tool_call_id TEXT NOT NULL UNIQUE,
    tool         TEXT NOT NULL,
    args         TEXT NOT NULL,
    status       TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    resolved_at  INTEGER,
    -- When a pending approval expires (created_at + APPROVAL_TTL_MS)
    expires_at   INTEGER,
    -- Who approved or rejected it (the local user's name); NULL when it
    -- expired or is pending
    decided_by   TEXT
);
CREATE INDEX approvals_status ON approvals(status);

-- Projection of `workflows/*.yaml` (global and agent-scoped)
CREATE TABLE workflows (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    definition         TEXT NOT NULL,
    created_at         INTEGER NOT NULL,
    -- The owning agent of an agent-scoped workflow
    agent_id           TEXT,
    updated_at         INTEGER,
    source_path        TEXT,
    projection_version INTEGER
);

-- No foreign key to `workflows`: that table is a projection and may be
-- rebuilt at any time; run history must survive it.
CREATE TABLE workflow_runs (
    id          TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    status      TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    context     TEXT NOT NULL DEFAULT '{}'
);

-- Projection of `.nandi/settings.yaml`, one row per top-level key
CREATE TABLE settings (
    key                TEXT PRIMARY KEY,
    value              TEXT NOT NULL,
    source_path        TEXT,
    projection_version INTEGER
);

-- Machine-local runtime settings (engine choice, canned script). Not in
-- the repo, never rebuilt.
CREATE TABLE local_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Projection of `<agents>/<id>/skill.md` + `instructions.md`
CREATE TABLE agents (
    id                 TEXT PRIMARY KEY,
    name               TEXT NOT NULL,
    description        TEXT NOT NULL DEFAULT '',
    -- `<provider>:<model>`; NULL uses the settings default
    model              TEXT,
    image              TEXT,
    -- Egress capabilities (JSON), for the Permissions panel
    capabilities       TEXT NOT NULL DEFAULT '{}',
    -- Tools, pilots, container limits, scoped workflows (JSON)
    definition         TEXT NOT NULL DEFAULT '{}',
    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL,
    source_path        TEXT NOT NULL,
    projection_version INTEGER NOT NULL
);

-- Projection of `.nandi/connectors.yaml`
CREATE TABLE connectors (
    id                 TEXT PRIMARY KEY,
    kind               TEXT NOT NULL,
    -- {config, secretRefs} (JSON); never secrets
    definition         TEXT NOT NULL,
    source_path        TEXT NOT NULL,
    projection_version INTEGER NOT NULL
);

-- What each projected entity was built from: its file and the file's
-- hash, so drift (a file changed while the app was closed) is detected
CREATE TABLE sync_state (
    entity_kind       TEXT NOT NULL,
    entity_id         TEXT NOT NULL,
    source_path       TEXT NOT NULL,
    file_hash         TEXT NOT NULL,
    last_projected_at INTEGER NOT NULL,
    PRIMARY KEY (entity_kind, entity_id)
);

-- Health of each agent's orchestrator (e.g. 'error' after a panic)
CREATE TABLE agent_state (
    agent_id    TEXT PRIMARY KEY,
    status      TEXT NOT NULL,
    error       TEXT,
    updated_at  INTEGER NOT NULL
);

-- Agent cards by what they serve; rebuilt from the agents
CREATE TABLE capability_index (
    entity_id   TEXT NOT NULL,
    capability  TEXT NOT NULL,
    agent_card  TEXT NOT NULL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (entity_id, capability)
);
CREATE INDEX idx_capability_index_capability
    ON capability_index(capability);

-- Cross-user tasks, both directions
CREATE TABLE delegations (
    id              TEXT PRIMARY KEY,
    -- 'outbound' (we asked) or 'inbound' (we served)
    direction       TEXT NOT NULL DEFAULT 'outbound',
    capability      TEXT NOT NULL,
    target_agent    TEXT NOT NULL,
    requester_agent TEXT NOT NULL,
    provider_owner  TEXT,
    requester_owner TEXT,
    input           TEXT NOT NULL DEFAULT 'null',
    output          TEXT,
    status          TEXT NOT NULL,
    started_at      INTEGER NOT NULL,
    ended_at        INTEGER,
    error           TEXT,
    tokens_in       INTEGER NOT NULL DEFAULT 0,
    tokens_out      INTEGER NOT NULL DEFAULT 0,
    -- The run that asked
    parent_run_id   TEXT
);
CREATE INDEX idx_delegations_started ON delegations(started_at);

-- Requests that came back while nobody watched (`a2a::returns`): Inbox
-- items until dismissed (`seen_at`)
CREATE TABLE request_returns (
    task_id         TEXT PRIMARY KEY,
    from_name       TEXT NOT NULL,
    provider_agent  TEXT NOT NULL,
    capability      TEXT NOT NULL,
    -- answered | done | declined | expired | failed
    outcome         TEXT NOT NULL,
    summary         TEXT NOT NULL DEFAULT '',
    request         TEXT NOT NULL DEFAULT '',
    returned_at     REAL NOT NULL,
    seen_at         REAL
);
CREATE INDEX idx_request_returns_returned ON request_returns(returned_at);

-- Relay connection state: instance ID, last seen online agents
CREATE TABLE relay_state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Who may call which local agent: allow | deny | ask
CREATE TABLE trust_policies (
    requester      TEXT NOT NULL,
    provider_agent TEXT NOT NULL,
    policy         TEXT NOT NULL,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (requester, provider_agent)
);

-- A cache of each agent's memory.md (the file is the gold copy)
CREATE TABLE agent_memory (
    agent_id   TEXT NOT NULL,
    idx        INTEGER NOT NULL,
    content    TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    source_run TEXT,
    PRIMARY KEY (agent_id, idx)
);

-- One row per attempt at a delegated task, on each side
CREATE TABLE tasks (
    id                  TEXT PRIMARY KEY,
    requester_instance  TEXT NOT NULL,
    requester_agent     TEXT NOT NULL,
    provider_instance   TEXT NOT NULL,
    provider_agent      TEXT NOT NULL,
    capability          TEXT NOT NULL,
    input               TEXT NOT NULL DEFAULT 'null',
    -- `TaskState` serde name (submitted, delivered, ..., abandoned)
    state               TEXT NOT NULL,
    eta_seconds         INTEGER,
    deadline            INTEGER,
    priority            INTEGER NOT NULL DEFAULT 0,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    acknowledged_at     INTEGER,
    completed_at        INTEGER,
    error               TEXT,
    result              TEXT,
    parent_task_id      TEXT,
    -- direct | gateway | failover
    source_kind         TEXT NOT NULL DEFAULT 'direct',
    attempt_number      INTEGER NOT NULL DEFAULT 1,
    previous_attempt_id TEXT
);
CREATE INDEX idx_tasks_state ON tasks(state);
CREATE INDEX idx_tasks_created ON tasks(created_at);

-- Every task transition, written with it
CREATE TABLE task_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     TEXT NOT NULL,
    from_state  TEXT,
    to_state    TEXT NOT NULL,
    reason      TEXT,
    -- The typed reason, and the stage the task was in
    reason_kind TEXT,
    stage       TEXT,
    at          INTEGER NOT NULL
);
CREATE INDEX idx_task_events_task ON task_events(task_id);

-- Progress, status, and clarification traffic
CREATE TABLE task_messages (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    kind    TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT 'null',
    at      INTEGER NOT NULL
);
CREATE INDEX idx_task_messages_task ON task_messages(task_id);

-- Provider side: results kept until the relay has them (a2a::result_outbox)
CREATE TABLE result_outbox (
    task_id TEXT PRIMARY KEY,
    result  TEXT NOT NULL,
    kept_at INTEGER NOT NULL
);

-- Completed task durations, for ETA estimates
CREATE TABLE eta_history (
    capability       TEXT NOT NULL,
    provider_agent   TEXT NOT NULL,
    duration_seconds INTEGER NOT NULL,
    at               INTEGER NOT NULL
);
CREATE INDEX idx_eta_history ON eta_history(capability, provider_agent, at);

-- Per-caller gateway allowances
CREATE TABLE gateway_callers (
    caller_instance      TEXT PRIMARY KEY,
    caller_owner         TEXT NOT NULL DEFAULT '',
    allowed_capabilities TEXT NOT NULL DEFAULT '[]',
    max_concurrent       INTEGER NOT NULL DEFAULT 1,
    monthly_task_budget  INTEGER,
    disabled             INTEGER NOT NULL DEFAULT 0,
    created_at           INTEGER NOT NULL
);

-- Gateway tasks per caller per month, `YYYY-MM`
CREATE TABLE gateway_usage (
    caller_instance TEXT NOT NULL,
    month           TEXT NOT NULL,
    task_count      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (caller_instance, month)
);

-- Tasks the relay delivered held for my approval by a group's policy:
-- a member's first request (`SealedTask.first_time_groups`, code
-- `first_request`) or a hold (`SealedTask.group_holds`, its code). Each
-- waits for my approval whatever my level (`services::groups::enforce`)
CREATE TABLE group_holds (
    task_id  TEXT NOT NULL,
    group_id TEXT NOT NULL,
    code     TEXT NOT NULL,
    PRIMARY KEY (task_id, group_id, code)
);

-- Gateway requests held for the owner's approval
CREATE TABLE gateway_holds (
    task_id         TEXT PRIMARY KEY,
    caller_instance TEXT NOT NULL,
    -- The caller's display name, as the gateway knew it
    caller_name     TEXT NOT NULL,
    capability      TEXT,
    -- The request in the caller's words (first lines)
    summary         TEXT NOT NULL DEFAULT '',
    -- pending | approved | rejected | expired
    status          TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    expires_at      INTEGER NOT NULL,
    decided_at      INTEGER
);
CREATE INDEX idx_gateway_holds_status ON gateway_holds(status);

-- Work assigned to the owner (`assign-me`), open until they end it
CREATE TABLE gateway_assignments (
    task_id         TEXT PRIMARY KEY,
    caller_instance TEXT NOT NULL,
    -- Who assigned it, as the gateway knew them
    caller_name     TEXT NOT NULL,
    -- What they asked, in their words
    request         TEXT NOT NULL DEFAULT '',
    -- open | done | declined | cancelled
    status          TEXT NOT NULL,
    -- The owner's note when they closed it
    note            TEXT,
    created_at      INTEGER NOT NULL,
    due_at          INTEGER,
    closed_at       INTEGER
);
CREATE INDEX idx_gateway_assignments_status ON gateway_assignments(status);

-- Links the requester attached to an assignment, in their order
CREATE TABLE gateway_assignment_links (
    task_id         TEXT NOT NULL REFERENCES gateway_assignments(task_id) ON DELETE CASCADE,
    position        INTEGER NOT NULL,
    name            TEXT NOT NULL,
    url             TEXT NOT NULL,
    PRIMARY KEY (task_id, position)
);

-- How we treat another member's instance
CREATE TABLE trust_relationships (
    instance_id           TEXT PRIMARY KEY,
    added_as_trusted_peer INTEGER NOT NULL DEFAULT 0,
    delegations_paused    INTEGER NOT NULL DEFAULT 0,
    last_updated          INTEGER NOT NULL
);

-- Other members' public keys, pinned on first sight
-- (`services::network::peer_keys`). `changed_*`: other keys the relay's
-- directory shows since; sending to them waits until the user accepts.
CREATE TABLE peer_keys (
    instance_id         TEXT PRIMARY KEY,
    signing_key         TEXT NOT NULL,
    sealing_key         TEXT NOT NULL,
    pinned_at           INTEGER NOT NULL,
    changed_signing_key TEXT,
    changed_sealing_key TEXT,
    changed_at          INTEGER
);

-- Another member's agents we blocked
CREATE TABLE blocked_members (
    instance_id TEXT NOT NULL,
    agent_id    TEXT NOT NULL,
    blocked_at  INTEGER NOT NULL,
    PRIMARY KEY (instance_id, agent_id)
);

-- Capability files (`.nandi/capabilities/<id>.yaml`), projected
CREATE TABLE capabilities_projection (
    id                 TEXT PRIMARY KEY,
    version            TEXT,
    name               TEXT,
    description        TEXT,
    owner_kind         TEXT,
    owner_id           TEXT,
    owner_display_name TEXT,
    definition_json    TEXT NOT NULL,
    source_path        TEXT,
    projection_version INTEGER,
    updated_at         INTEGER
);

-- Who provides what, here and on other instances
CREATE TABLE capability_providers (
    capability_id      TEXT NOT NULL,
    capability_version TEXT,
    instance_id        TEXT NOT NULL,
    -- agent | workflow
    provider_kind      TEXT NOT NULL,
    provider_id        TEXT NOT NULL,
    enabled            INTEGER DEFAULT 1,
    max_concurrent     INTEGER,
    -- JSON array of tool IDs
    provided_tools     TEXT,
    registered_at      INTEGER,
    PRIMARY KEY (capability_id, instance_id, provider_id)
);

-- One row per finished task per side
CREATE TABLE capability_evidence (
    id                   TEXT PRIMARY KEY,
    capability_id        TEXT NOT NULL,
    capability_version   TEXT,
    task_id              TEXT NOT NULL,
    provider_instance    TEXT,
    provider_kind        TEXT,
    provider_id          TEXT,
    -- requester | provider
    role                 TEXT NOT NULL,
    status               TEXT NOT NULL,
    duration_ms          INTEGER,
    tokens_in            INTEGER,
    tokens_out           INTEGER,
    cost_usd             REAL,
    clarification_rounds INTEGER DEFAULT 0,
    escalation_required  INTEGER DEFAULT 0,
    user_rating          INTEGER,
    -- JSON, for workflow providers
    step_breakdown       TEXT,
    observed_at          INTEGER NOT NULL,
    metadata             TEXT
);
CREATE INDEX idx_ev_cap_provider
    ON capability_evidence(capability_id, provider_instance, provider_id);
CREATE INDEX idx_ev_time ON capability_evidence(capability_id, observed_at);

-- The unified tool catalog
CREATE TABLE tool_catalog (
    id              TEXT PRIMARY KEY,
    definition_json TEXT NOT NULL,
    -- builtin | connector | pilot | custom
    source          TEXT NOT NULL,
    source_id       TEXT,
    enabled         INTEGER DEFAULT 1,
    updated_at      INTEGER
);

-- Cached rolling aggregates. Key columns are '' rather than NULL (NULLs
-- never collide in a primary key, so upserts would duplicate).
CREATE TABLE capability_aggregates (
    capability_id     TEXT NOT NULL,
    provider_instance TEXT NOT NULL DEFAULT '',
    provider_kind     TEXT NOT NULL DEFAULT '',
    provider_id       TEXT NOT NULL DEFAULT '',
    window_days       INTEGER NOT NULL,
    aggregate_json    TEXT NOT NULL,
    computed_at       INTEGER,
    PRIMARY KEY (capability_id, provider_instance, provider_kind, provider_id, window_days)
);

-- Clusters of clarification questions per capability
CREATE TABLE capability_ambiguities (
    id                 TEXT PRIMARY KEY,
    capability_id      TEXT NOT NULL,
    cluster_label      TEXT NOT NULL,
    -- JSON array
    sample_questions   TEXT,
    occurrence_count   INTEGER,
    first_seen_at      INTEGER,
    last_seen_at       INTEGER,
    promoted_to_schema INTEGER DEFAULT 0
);
CREATE INDEX idx_ambiguities_cap ON capability_ambiguities(capability_id);

-- Consecutive failures per provider and capability
CREATE TABLE provider_circuits (
    instance_id          TEXT NOT NULL,
    provider_id          TEXT NOT NULL,
    capability_id        TEXT NOT NULL,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    opened_at            INTEGER,
    PRIMARY KEY (instance_id, provider_id, capability_id)
);

-- Harness sessions and their events
CREATE TABLE harness_sessions (
    id              TEXT PRIMARY KEY,
    adapter_id      TEXT NOT NULL,
    agent_id        TEXT,
    run_id          TEXT,
    acp_session_id  TEXT,
    -- starting | running | finished | failed | killed
    status          TEXT NOT NULL,
    child_pid       INTEGER,
    container_name  TEXT,
    working_dir     TEXT NOT NULL,
    model           TEXT,
    provider        TEXT,
    stop_reason     TEXT,
    error           TEXT,
    input_tokens    INTEGER NOT NULL DEFAULT 0,
    output_tokens   INTEGER NOT NULL DEFAULT 0,
    started_at      INTEGER NOT NULL,
    ended_at        INTEGER
);
CREATE INDEX harness_sessions_agent ON harness_sessions(agent_id, started_at);
CREATE INDEX harness_sessions_run ON harness_sessions(run_id);

CREATE TABLE harness_events (
    session_id  TEXT NOT NULL REFERENCES harness_sessions(id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,
    kind        TEXT NOT NULL,
    payload     TEXT NOT NULL DEFAULT '{}',
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (session_id, seq)
);

-- Harness CLIs found on this machine
CREATE TABLE detected_harnesses (
    harness_id          TEXT PRIMARY KEY,
    installed           INTEGER NOT NULL DEFAULT 0,
    executable          TEXT,
    version             TEXT,
    config_dir          TEXT,
    configured_provider TEXT,
    authenticated       INTEGER NOT NULL DEFAULT 0,
    error               TEXT,
    detected_at         INTEGER NOT NULL
);

-- Machine-local harness preferences
CREATE TABLE harness_preferences (
    harness_id      TEXT PRIMARY KEY,
    is_default      INTEGER NOT NULL DEFAULT 0,
    default_model   TEXT,
    -- custom:<id> harnesses: the executable and its arguments (JSON array)
    command         TEXT,
    args            TEXT NOT NULL DEFAULT '[]',
    updated_at      INTEGER NOT NULL
);

-- A cache of `aws configure` output: the AWS config files stay the source
CREATE TABLE aws_profiles (
    name                TEXT PRIMARY KEY,
    kind                TEXT NOT NULL,
    region              TEXT,
    sso_session         TEXT,
    sso_start_url       TEXT,
    sso_region          TEXT,
    sso_account_id      TEXT,
    sso_role_name       TEXT,
    role_arn            TEXT,
    source_profile      TEXT,
    credential_source   TEXT,
    credential_valid    INTEGER,
    credential_expires_at INTEGER,
    last_checked_at     INTEGER,
    updated_at          INTEGER NOT NULL
);

-- A cache of the Bedrock models AWS lists per region
CREATE TABLE bedrock_models_cache (
    region      TEXT NOT NULL,
    model_id    TEXT NOT NULL,
    kind        TEXT NOT NULL,
    provider    TEXT NOT NULL,
    name        TEXT NOT NULL,
    model_json  TEXT NOT NULL DEFAULT '{}',
    fetched_at  INTEGER NOT NULL,
    PRIMARY KEY (region, model_id)
);

-- What was asked in the ask box and what came back
CREATE TABLE ask_history (
    id               TEXT PRIMARY KEY,
    request_text     TEXT NOT NULL,
    -- `AskContext` JSON
    context          TEXT,
    -- question | action | edit | clarification
    response_kind    TEXT NOT NULL,
    -- `AskResponse` JSON
    response_payload TEXT NOT NULL,
    at               INTEGER NOT NULL
);
CREATE INDEX idx_ask_at ON ask_history(at);

-- Machine-local Work view state (e.g. paused workflow runs)
CREATE TABLE work_preferences (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Manual workflow steps waiting for a person
CREATE TABLE manual_steps (
    run_id       TEXT NOT NULL,
    step_id      TEXT NOT NULL,
    workflow_id  TEXT NOT NULL,
    -- pending | completed | cancelled
    status       TEXT NOT NULL,
    instructions TEXT NOT NULL DEFAULT '',
    deadline     INTEGER,
    created_at   INTEGER NOT NULL,
    completed_at INTEGER,
    PRIMARY KEY (run_id, step_id)
);
CREATE INDEX idx_manual_steps_status ON manual_steps(status);

-- Tasks handed from one provider to another
CREATE TABLE reassigned_tasks (
    task_id       TEXT NOT NULL,
    from_instance TEXT NOT NULL,
    to_instance   TEXT NOT NULL,
    reason        TEXT,
    reassigned_at INTEGER NOT NULL,
    PRIMARY KEY (task_id, to_instance)
);

-- The last-read repo LLM config
CREATE TABLE repo_config_cache (
    key       TEXT PRIMARY KEY,
    value     TEXT NOT NULL,
    synced_at INTEGER NOT NULL
);

-- Local asks saved as Requests, with their Steps linked to the run,
-- workflow run or task that does each
CREATE TABLE requests (
    id         TEXT PRIMARY KEY,
    title      TEXT NOT NULL,
    -- me (the ask box, Studio) | email | calendar | nandi (work the
    -- Nandi started itself)
    source     TEXT NOT NULL,
    -- What was asked, word for word
    ask_text   TEXT NOT NULL DEFAULT '',
    -- `ActionPlan` JSON
    plan       TEXT,
    -- planning | running | waiting | done | failed | cancelled
    status     TEXT NOT NULL,
    error      TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    ended_at   INTEGER
);
CREATE INDEX idx_requests_status ON requests(status, updated_at);

CREATE TABLE request_steps (
    request_id      TEXT NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
    -- 0-based, the plan's order
    position        INTEGER NOT NULL,
    -- localAgent | capability | workflow | human | manual
    kind            TEXT NOT NULL,
    target          TEXT NOT NULL,
    label           TEXT NOT NULL,
    -- JSON array of positions that must be done first
    depends_on      TEXT NOT NULL DEFAULT '[]',
    -- pending | running | waiting | done | failed | skipped
    status          TEXT NOT NULL,
    run_id          TEXT,
    workflow_run_id TEXT,
    task_id         TEXT,
    error           TEXT,
    started_at      INTEGER,
    ended_at        INTEGER,
    PRIMARY KEY (request_id, position)
);
CREATE INDEX idx_request_steps_run ON request_steps(run_id);
CREATE INDEX idx_request_steps_workflow_run ON request_steps(workflow_run_id);

-- News Nandi added to a Request while it ran
CREATE TABLE request_updates (
    request_id TEXT NOT NULL REFERENCES requests(id) ON DELETE CASCADE,
    at         INTEGER NOT NULL,
    -- email | calendar | nandi
    source     TEXT NOT NULL,
    text       TEXT NOT NULL
);
CREATE INDEX idx_request_updates_at ON request_updates(at);

-- Work from outside (mail, calendar), screened and waiting for the user's
-- approval before it starts or reaches a Request (`services::intake`)
CREATE TABLE intake_queue (
    id                TEXT PRIMARY KEY,
    at                INTEGER NOT NULL,
    -- email | calendar (`RequestSource`)
    source            TEXT NOT NULL,
    -- start | update
    action            TEXT NOT NULL,
    request_id        TEXT,
    request_title     TEXT,
    text              TEXT NOT NULL,
    sender            TEXT,
    -- `Screening` JSON
    screening         TEXT NOT NULL,
    -- pending | approved | dismissed
    status            TEXT NOT NULL,
    result_request_id TEXT,
    -- Its Nandi thread (`intake:<id>`, a conversation)
    thread_id         TEXT NOT NULL,
    -- `ActionPlan` JSON, for `start`: planned when it was queued
    plan              TEXT
);
CREATE INDEX idx_intake_queue_status ON intake_queue(status, at);
CREATE INDEX idx_intake_queue_thread ON intake_queue(thread_id);
CREATE INDEX idx_request_steps_task ON request_steps(task_id);

-- A run's plan, step by step (`services::run_plan`): run data, never the
-- gold copy. Each `plan_set` adds a revision; `plan_update` changes the
-- latest one.
CREATE TABLE run_plans (
    run_id     TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
    revision   INTEGER NOT NULL,
    -- `RunPlanStep[]` JSON
    steps      TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (run_id, revision)
);
