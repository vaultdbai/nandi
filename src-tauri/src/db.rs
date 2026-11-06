//! SQLite persistence at `~/.nandi/state.db`.
//!
//! Two kinds of table live here:
//! - Runtime data, owned by SQLite: conversations, messages, runs, the
//!   agent queue, approvals, run events, workflow runs, trigger fires,
//!   agent health, and `local_settings`.
//! - The projection of the gold copy (`agents`, `workflows`, `triggers`,
//!   `settings`, `connectors`, plus `sync_state`): a rebuildable view of the
//!   repo files, written only by `services::projection`. Projected rows
//!   carry `source_path` and `projection_version`.
//!
//! The UI reconstructs everything from snapshots of these tables plus event
//! deltas.
//!
//! JSON columns are declared `TEXT`: a column declared `JSON` gets NUMERIC
//! affinity in SQLite, which would coerce a JSON value like `"123"`.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use r2d2::{Pool, PooledConnection};
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{
    ApprovalStatus, AttachmentInfo, ChatMessage, PendingApproval, QueueEntry, RunEventRecord,
    RunStatus, RunSummary, SourceType, StudioConversationSummary, Timestamp, ToolCallStatus,
    WorkflowSummary, STUDIO_AGENT_ID,
};

/// Raised when a table changes. Nothing has been released, so there are no
/// migrations: a database from another version is refused (`init_db`).
pub const SCHEMA_VERSION: i64 = 3;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS conversations (
    id          TEXT PRIMARY KEY,
    agent_id    TEXT NOT NULL,
    created_at  INTEGER NOT NULL,
    title       TEXT
);
CREATE INDEX IF NOT EXISTS conversations_agent ON conversations(agent_id);

CREATE TABLE IF NOT EXISTS messages (
    id              TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
    run_id          TEXT,
    role            TEXT NOT NULL,
    content         TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    metadata        TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS messages_conversation ON messages(conversation_id, created_at);
CREATE INDEX IF NOT EXISTS messages_run ON messages(run_id);

CREATE TABLE IF NOT EXISTS runs (
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
CREATE INDEX IF NOT EXISTS runs_agent ON runs(agent_id, started_at);
CREATE INDEX IF NOT EXISTS runs_status ON runs(status);

-- Runs waiting for their agent's current turn, with the input to start
-- them with. A row is deleted when its run starts or is cancelled.
CREATE TABLE IF NOT EXISTS agent_queue (
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
CREATE INDEX IF NOT EXISTS agent_queue_order ON agent_queue(agent_id, priority, created_at);

-- Projection of `triggers/<id>.yaml`: rules that start agent or workflow
-- runs when something happens. `source_config` never holds a secret (it's
-- in the keyring).
CREATE TABLE IF NOT EXISTS triggers (
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
CREATE INDEX IF NOT EXISTS triggers_enabled ON triggers(enabled, source_type);

-- Runtime state of each trigger, kept when the projection is rebuilt
CREATE TABLE IF NOT EXISTS trigger_state (
    trigger_id    TEXT PRIMARY KEY,
    last_fired_at INTEGER
);

-- Every time a trigger fired, whether or not it dispatched a run
CREATE TABLE IF NOT EXISTS trigger_fires (
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
CREATE INDEX IF NOT EXISTS trigger_fires_trigger ON trigger_fires(trigger_id, fired_at DESC);

CREATE TABLE IF NOT EXISTS run_events (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id      TEXT NOT NULL,
    kind        TEXT NOT NULL,
    payload     TEXT NOT NULL,
    created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS run_events_run ON run_events(run_id);

CREATE TABLE IF NOT EXISTS approvals (
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
CREATE INDEX IF NOT EXISTS approvals_status ON approvals(status);

-- Projection of `workflows/*.yaml` (global and agent-scoped)
CREATE TABLE IF NOT EXISTS workflows (
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
CREATE TABLE IF NOT EXISTS workflow_runs (
    id          TEXT PRIMARY KEY,
    workflow_id TEXT NOT NULL,
    status      TEXT NOT NULL,
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    context     TEXT NOT NULL DEFAULT '{}'
);

-- Projection of `.nandi/settings.yaml`, one row per top-level key
CREATE TABLE IF NOT EXISTS settings (
    key                TEXT PRIMARY KEY,
    value              TEXT NOT NULL,
    source_path        TEXT,
    projection_version INTEGER
);

-- Machine-local runtime settings (engine choice, canned script). Not in
-- the repo, never rebuilt.
CREATE TABLE IF NOT EXISTS local_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Projection of `<agents>/<id>/skill.md` + `instructions.md`
CREATE TABLE IF NOT EXISTS agents (
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
CREATE TABLE IF NOT EXISTS connectors (
    id                 TEXT PRIMARY KEY,
    kind               TEXT NOT NULL,
    -- {config, secretRefs} (JSON); never secrets
    definition         TEXT NOT NULL,
    source_path        TEXT NOT NULL,
    projection_version INTEGER NOT NULL
);

-- What each projected entity was built from: its file and the file's
-- hash, so drift (a file changed while the app was closed) is detected
CREATE TABLE IF NOT EXISTS sync_state (
    entity_kind       TEXT NOT NULL,
    entity_id         TEXT NOT NULL,
    source_path       TEXT NOT NULL,
    file_hash         TEXT NOT NULL,
    last_projected_at INTEGER NOT NULL,
    PRIMARY KEY (entity_kind, entity_id)
);

-- Health of each agent's orchestrator (e.g. 'error' after a panic)
CREATE TABLE IF NOT EXISTS agent_state (
    agent_id    TEXT PRIMARY KEY,
    status      TEXT NOT NULL,
    error       TEXT,
    updated_at  INTEGER NOT NULL
);

-- Agent cards by what they serve; rebuilt from the agents
CREATE TABLE IF NOT EXISTS capability_index (
    entity_id   TEXT NOT NULL,
    capability  TEXT NOT NULL,
    agent_card  TEXT NOT NULL,
    updated_at  INTEGER NOT NULL,
    PRIMARY KEY (entity_id, capability)
);
CREATE INDEX IF NOT EXISTS idx_capability_index_capability
    ON capability_index(capability);

-- Teams and their private registries
CREATE TABLE IF NOT EXISTS teams (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    github_org    TEXT NOT NULL,
    registry_repo TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    -- 'github' (the org) or 'entra' (the group ID)
    source_kind       TEXT NOT NULL DEFAULT 'github',
    source_identifier TEXT
);
CREATE TABLE IF NOT EXISTS team_members (
    team_id       TEXT NOT NULL,
    github_handle TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    role          TEXT NOT NULL,
    joined_at     INTEGER NOT NULL,
    PRIMARY KEY (team_id, github_handle)
);

-- Cross-user tasks, both directions
CREATE TABLE IF NOT EXISTS delegations (
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
CREATE INDEX IF NOT EXISTS idx_delegations_started ON delegations(started_at);

-- Requests that came back while nobody watched (`a2a::returns`): Inbox
-- items until dismissed (`seen_at`)
CREATE TABLE IF NOT EXISTS request_returns (
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
CREATE INDEX IF NOT EXISTS idx_request_returns_returned ON request_returns(returned_at);

-- Relay connection state: instance ID, last seen online agents
CREATE TABLE IF NOT EXISTS relay_state (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Who may call which local agent: allow | deny | ask
CREATE TABLE IF NOT EXISTS trust_policies (
    requester      TEXT NOT NULL,
    provider_agent TEXT NOT NULL,
    policy         TEXT NOT NULL,
    updated_at     INTEGER NOT NULL,
    PRIMARY KEY (requester, provider_agent)
);

-- A cache of each agent's memory.md (the file is the gold copy)
CREATE TABLE IF NOT EXISTS agent_memory (
    agent_id   TEXT NOT NULL,
    idx        INTEGER NOT NULL,
    content    TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    source_run TEXT,
    PRIMARY KEY (agent_id, idx)
);

-- One row per attempt at a delegated task, on each side
CREATE TABLE IF NOT EXISTS tasks (
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
CREATE INDEX IF NOT EXISTS idx_tasks_state ON tasks(state);
CREATE INDEX IF NOT EXISTS idx_tasks_created ON tasks(created_at);

-- Every task transition, written with it
CREATE TABLE IF NOT EXISTS task_events (
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
CREATE INDEX IF NOT EXISTS idx_task_events_task ON task_events(task_id);

-- Progress, status, and clarification traffic
CREATE TABLE IF NOT EXISTS task_messages (
    id      INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    kind    TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT 'null',
    at      INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_task_messages_task ON task_messages(task_id);

-- Completed task durations, for ETA estimates
CREATE TABLE IF NOT EXISTS eta_history (
    capability       TEXT NOT NULL,
    provider_agent   TEXT NOT NULL,
    duration_seconds INTEGER NOT NULL,
    at               INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_eta_history ON eta_history(capability, provider_agent, at);

-- Per-caller gateway allowances
CREATE TABLE IF NOT EXISTS gateway_callers (
    caller_instance      TEXT PRIMARY KEY,
    caller_owner         TEXT NOT NULL DEFAULT '',
    allowed_capabilities TEXT NOT NULL DEFAULT '[]',
    max_concurrent       INTEGER NOT NULL DEFAULT 1,
    monthly_task_budget  INTEGER,
    disabled             INTEGER NOT NULL DEFAULT 0,
    created_at           INTEGER NOT NULL
);

-- Gateway tasks per caller per month, `YYYY-MM`
CREATE TABLE IF NOT EXISTS gateway_usage (
    caller_instance TEXT NOT NULL,
    month           TEXT NOT NULL,
    task_count      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (caller_instance, month)
);

-- Gateway requests held for the owner's approval
CREATE TABLE IF NOT EXISTS gateway_holds (
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
CREATE INDEX IF NOT EXISTS idx_gateway_holds_status ON gateway_holds(status);

-- Work assigned to the owner (`assign-me`), open until they end it
CREATE TABLE IF NOT EXISTS gateway_assignments (
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
CREATE INDEX IF NOT EXISTS idx_gateway_assignments_status ON gateway_assignments(status);

-- Links the requester attached to an assignment, in their order
CREATE TABLE IF NOT EXISTS gateway_assignment_links (
    task_id         TEXT NOT NULL REFERENCES gateway_assignments(task_id) ON DELETE CASCADE,
    position        INTEGER NOT NULL,
    name            TEXT NOT NULL,
    url             TEXT NOT NULL,
    PRIMARY KEY (task_id, position)
);

-- How we treat another member's instance
CREATE TABLE IF NOT EXISTS trust_relationships (
    instance_id           TEXT PRIMARY KEY,
    added_as_trusted_peer INTEGER NOT NULL DEFAULT 0,
    delegations_paused    INTEGER NOT NULL DEFAULT 0,
    last_updated          INTEGER NOT NULL
);

-- Other members' public keys, pinned on first sight
-- (`services::network::peer_keys`). `changed_*`: other keys the relay's
-- directory shows since; sending to them waits until the user accepts.
CREATE TABLE IF NOT EXISTS peer_keys (
    instance_id         TEXT PRIMARY KEY,
    signing_key         TEXT NOT NULL,
    sealing_key         TEXT NOT NULL,
    pinned_at           INTEGER NOT NULL,
    changed_signing_key TEXT,
    changed_sealing_key TEXT,
    changed_at          INTEGER
);

-- Another member's agents we blocked
CREATE TABLE IF NOT EXISTS blocked_members (
    instance_id TEXT NOT NULL,
    agent_id    TEXT NOT NULL,
    blocked_at  INTEGER NOT NULL,
    PRIMARY KEY (instance_id, agent_id)
);

-- Member profiles per team, cached for 60s
CREATE TABLE IF NOT EXISTS roster_cache (
    team_id      TEXT NOT NULL,
    member_handle TEXT NOT NULL,
    profile_json TEXT NOT NULL,
    cached_at    INTEGER NOT NULL,
    PRIMARY KEY (team_id, member_handle)
);

-- Capability files (`.nandi/capabilities/<id>.yaml`) and registry
-- definitions, projected. `registry_origin` is NULL for local ones.
CREATE TABLE IF NOT EXISTS capabilities_projection (
    id                 TEXT PRIMARY KEY,
    version            TEXT,
    name               TEXT,
    description        TEXT,
    owner_kind         TEXT,
    owner_id           TEXT,
    owner_display_name TEXT,
    definition_json    TEXT NOT NULL,
    source_path        TEXT,
    registry_origin    TEXT,
    projection_version INTEGER,
    updated_at         INTEGER
);

-- Who provides what, here and on other instances
CREATE TABLE IF NOT EXISTS capability_providers (
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

-- Subscribed git registries
CREATE TABLE IF NOT EXISTS capability_registries (
    id             TEXT PRIMARY KEY,
    url            TEXT NOT NULL UNIQUE,
    -- local | github | gitlab | generic
    kind           TEXT NOT NULL,
    branch_prefix  TEXT,
    user_slug      TEXT,
    last_synced_at INTEGER,
    last_commit    TEXT,
    subscribed     INTEGER DEFAULT 1
);

-- One row per finished task per side
CREATE TABLE IF NOT EXISTS capability_evidence (
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
    approval_required    INTEGER DEFAULT 0,
    user_rating          INTEGER,
    -- JSON, for workflow providers
    step_breakdown       TEXT,
    observed_at          INTEGER NOT NULL,
    metadata             TEXT
);
CREATE INDEX IF NOT EXISTS idx_ev_cap_provider
    ON capability_evidence(capability_id, provider_instance, provider_id);
CREATE INDEX IF NOT EXISTS idx_ev_time ON capability_evidence(capability_id, observed_at);

-- The unified tool catalog
CREATE TABLE IF NOT EXISTS tool_catalog (
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
CREATE TABLE IF NOT EXISTS capability_aggregates (
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
CREATE TABLE IF NOT EXISTS capability_ambiguities (
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
CREATE INDEX IF NOT EXISTS idx_ambiguities_cap ON capability_ambiguities(capability_id);

-- Consecutive failures per provider and capability
CREATE TABLE IF NOT EXISTS provider_circuits (
    instance_id          TEXT NOT NULL,
    provider_id          TEXT NOT NULL,
    capability_id        TEXT NOT NULL,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    opened_at            INTEGER,
    PRIMARY KEY (instance_id, provider_id, capability_id)
);

-- Harness sessions and their events
CREATE TABLE IF NOT EXISTS harness_sessions (
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
CREATE INDEX IF NOT EXISTS harness_sessions_agent ON harness_sessions(agent_id, started_at);
CREATE INDEX IF NOT EXISTS harness_sessions_run ON harness_sessions(run_id);

CREATE TABLE IF NOT EXISTS harness_events (
    session_id  TEXT NOT NULL REFERENCES harness_sessions(id) ON DELETE CASCADE,
    seq         INTEGER NOT NULL,
    kind        TEXT NOT NULL,
    payload     TEXT NOT NULL DEFAULT '{}',
    created_at  INTEGER NOT NULL,
    PRIMARY KEY (session_id, seq)
);

-- Harness CLIs found on this machine
CREATE TABLE IF NOT EXISTS detected_harnesses (
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
CREATE TABLE IF NOT EXISTS harness_preferences (
    harness_id      TEXT PRIMARY KEY,
    is_default      INTEGER NOT NULL DEFAULT 0,
    default_model   TEXT,
    -- custom:<id> harnesses: the executable and its arguments (JSON array)
    command         TEXT,
    args            TEXT NOT NULL DEFAULT '[]',
    updated_at      INTEGER NOT NULL
);

-- A cache of `aws configure` output: the AWS config files stay the source
CREATE TABLE IF NOT EXISTS aws_profiles (
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
CREATE TABLE IF NOT EXISTS bedrock_models_cache (
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
CREATE TABLE IF NOT EXISTS ask_history (
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
CREATE INDEX IF NOT EXISTS idx_ask_at ON ask_history(at);

-- Machine-local Work view state (e.g. paused workflow runs)
CREATE TABLE IF NOT EXISTS work_preferences (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

-- Manual workflow steps waiting for a person
CREATE TABLE IF NOT EXISTS manual_steps (
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
CREATE INDEX IF NOT EXISTS idx_manual_steps_status ON manual_steps(status);

-- Tasks handed from one provider to another
CREATE TABLE IF NOT EXISTS reassigned_tasks (
    task_id       TEXT NOT NULL,
    from_instance TEXT NOT NULL,
    to_instance   TEXT NOT NULL,
    reason        TEXT,
    reassigned_at INTEGER NOT NULL,
    PRIMARY KEY (task_id, to_instance)
);

-- The last-read repo LLM config
CREATE TABLE IF NOT EXISTS repo_config_cache (
    key       TEXT PRIMARY KEY,
    value     TEXT NOT NULL,
    synced_at INTEGER NOT NULL
);

-- Local asks saved as Requests, with their Steps linked to the run,
-- workflow run or task that does each
CREATE TABLE IF NOT EXISTS requests (
    id         TEXT PRIMARY KEY,
    title      TEXT NOT NULL,
    -- me (the ask box) for now; trigger | schedule | workflow | peer later
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
CREATE INDEX IF NOT EXISTS idx_requests_status ON requests(status, updated_at);

CREATE TABLE IF NOT EXISTS request_steps (
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
CREATE INDEX IF NOT EXISTS idx_request_steps_run ON request_steps(run_id);
CREATE INDEX IF NOT EXISTS idx_request_steps_workflow_run ON request_steps(workflow_run_id);
CREATE INDEX IF NOT EXISTS idx_request_steps_task ON request_steps(task_id);
"#;

pub type DbConn = PooledConnection<SqliteConnectionManager>;

/// Pooled handle to the state database. Cheap to clone.
#[derive(Clone)]
pub struct Db {
    pool: Pool<SqliteConnectionManager>,
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default()
}

pub fn ts(ms: i64) -> Timestamp {
    ms as f64
}

impl Db {
    /// Opens (creating if needed) the database and applies the schema.
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
        }
        let manager = SqliteConnectionManager::file(path).with_init(configure_connection);
        let pool = Pool::builder()
            .max_size(8)
            .build(manager)
            .map_err(|e| format!("Failed to open {}: {e}", path.display()))?;
        let db = Self { pool };
        {
            let conn = db.conn()?;
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| format!("Failed to enable WAL: {e}"))?;
        }
        db.init()?;
        Ok(db)
    }

    /// A private in-memory database (one connection, so every caller sees
    /// the same data).
    #[cfg(test)]
    pub fn in_memory() -> Self {
        let manager = SqliteConnectionManager::memory().with_init(configure_connection);
        let pool = Pool::builder().max_size(1).build(manager).unwrap();
        let db = Self { pool };
        db.init().unwrap();
        db
    }

    pub fn conn(&self) -> Result<DbConn, String> {
        self.pool
            .get()
            .map_err(|e| format!("Database unavailable: {e}"))
    }

    fn init(&self) -> Result<(), String> {
        let conn = self.conn()?;
        init_db(&conn).map_err(|e| format!("Failed to initialize database: {e}"))
    }

    /// Runs blocking DB work off the async runtime.
    pub async fn call<T, F>(&self, f: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool
                .get()
                .map_err(|e| format!("Database unavailable: {e}"))?;
            f(&mut conn).map_err(|e| format!("Database error: {e}"))
        })
        .await
        .map_err(|e| format!("Database task failed: {e}"))?
    }
}

fn configure_connection(conn: &mut Connection) -> rusqlite::Result<()> {
    conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;")
}

/// Creates any missing tables, stamps the schema version and seeds default
/// settings. Safe to run on every open. A database from another schema
/// version is refused, with what to do about it.
pub fn init_db(conn: &Connection) -> Result<(), String> {
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| e.to_string())?;
    if version != 0 && version != SCHEMA_VERSION {
        return Err(format!(
            "The local database is from another Nandi build (schema {version}, this build \
             uses {SCHEMA_VERSION}). Quit Nandi, delete ~/.nandi/local/state.db with its \
             -wal and -shm files, and start it again: your agents and workflows are in \
             ~/.nandi/repo and aren't touched."
        ));
    }
    init_schema(conn).map_err(|e| e.to_string())
}

fn init_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)?;
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    conn.execute(
        "INSERT OR IGNORE INTO local_settings (key, value) VALUES (?1, ?2)",
        params![
            crate::engine::CANNED_SCRIPT_KEY,
            crate::engine::default_script().to_string()
        ],
    )?;
    Ok(())
}

fn json_col(row: &Row, index: usize) -> rusqlite::Result<Value> {
    let text: String = row.get(index)?;
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

// ---------------------------------------------------------------------------
// Message metadata (per role)
// ---------------------------------------------------------------------------

pub mod role {
    pub const USER: &str = "user";
    pub const ASSISTANT: &str = "assistant";
    pub const TOOL_CALL: &str = "tool_call";
    pub const APPROVAL: &str = "approval";
    pub const ERROR: &str = "error";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMeta {
    #[serde(default)]
    pub attachments: Vec<AttachmentInfo>,
}

/// A tool call the engine planned in an assistant step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedCall {
    pub id: String,
    pub tool: String,
    pub args: Value,
    pub requires_approval: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMeta {
    pub step: u32,
    #[serde(default)]
    pub planned_calls: Vec<PlannedCall>,
    /// The model's thinking for this step (replayed to DeepSeek)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallMeta {
    pub tool: String,
    pub args: Value,
    pub status: ToolCallStatus,
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalMeta {
    pub tool: String,
    pub args: Value,
    pub status: ApprovalStatus,
    /// Epoch ms when a pending approval expires
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

// ---------------------------------------------------------------------------
// Conversations
// ---------------------------------------------------------------------------

/// Each agent has one long-lived conversation with a deterministic ID.
pub fn agent_conversation_id(agent_id: &str) -> String {
    format!("agent:{agent_id}")
}

pub fn ensure_agent_conversation(conn: &Connection, agent_id: &str) -> rusqlite::Result<String> {
    let id = agent_conversation_id(agent_id);
    conn.execute(
        "INSERT OR IGNORE INTO conversations (id, agent_id, created_at, title) VALUES (?1, ?2, ?3, NULL)",
        params![id, agent_id, now_ms()],
    )?;
    Ok(id)
}

/// `email:<check id>`: one email check's own conversation, so its turn
/// sees neither the user's chats nor earlier checks' mail, and later
/// chats never see this mail in their history
pub fn email_conversation_id(check_id: &str) -> String {
    format!("email:{check_id}")
}

pub fn ensure_email_conversation(
    conn: &Connection,
    agent_id: &str,
    check_id: &str,
    title: &str,
) -> rusqlite::Result<String> {
    let id = email_conversation_id(check_id);
    conn.execute(
        "INSERT OR IGNORE INTO conversations (id, agent_id, created_at, title) VALUES (?1, ?2, ?3, ?4)",
        params![id, agent_id, now_ms(), title],
    )?;
    Ok(id)
}

/// `task:<task id>`: a delegated task's own conversation, so its turn sees
/// neither the owner's chats with the agent nor other requesters' tasks
pub fn task_conversation_id(task_id: &str) -> String {
    format!("task:{task_id}")
}

pub fn ensure_task_conversation(
    conn: &Connection,
    agent_id: &str,
    task_id: &str,
    title: &str,
) -> rusqlite::Result<String> {
    let id = task_conversation_id(task_id);
    conn.execute(
        "INSERT OR IGNORE INTO conversations (id, agent_id, created_at, title) VALUES (?1, ?2, ?3, ?4)",
        params![id, agent_id, now_ms(), title],
    )?;
    Ok(id)
}

pub fn insert_studio_conversation(
    conn: &Connection,
    id: &str,
    title: &str,
) -> rusqlite::Result<StudioConversationSummary> {
    let now = now_ms();
    conn.execute(
        "INSERT INTO conversations (id, agent_id, created_at, title) VALUES (?1, ?2, ?3, ?4)",
        params![id, STUDIO_AGENT_ID, now, title],
    )?;
    Ok(StudioConversationSummary {
        id: id.to_string(),
        title: title.to_string(),
        created_at: ts(now),
        updated_at: ts(now),
        last_message: None,
    })
}

/// Titles a conversation that has none yet. Returns whether it did.
pub fn set_title_if_untitled(conn: &Connection, id: &str, title: &str) -> rusqlite::Result<bool> {
    let changed = conn.execute(
        "UPDATE conversations SET title = ?2 WHERE id = ?1 AND COALESCE(title, '') = ''",
        params![id, title],
    )?;
    Ok(changed > 0)
}

pub fn conversation_agent(
    conn: &Connection,
    conversation_id: &str,
) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT agent_id FROM conversations WHERE id = ?1",
        [conversation_id],
        |row| row.get(0),
    )
    .optional()
}

pub fn list_studio_conversations(
    conn: &Connection,
) -> rusqlite::Result<Vec<StudioConversationSummary>> {
    let mut stmt = conn.prepare(
        "SELECT c.id, COALESCE(c.title, ''), c.created_at,
                COALESCE((SELECT MAX(created_at) FROM messages m WHERE m.conversation_id = c.id), c.created_at),
                (SELECT content FROM messages m
                  WHERE m.conversation_id = c.id AND m.role IN ('user', 'assistant')
                  ORDER BY m.created_at DESC, m.rowid DESC LIMIT 1)
           FROM conversations c
          WHERE c.agent_id = ?1
          ORDER BY 4 DESC",
    )?;
    let rows = stmt.query_map([STUDIO_AGENT_ID], |row| {
        Ok(StudioConversationSummary {
            id: row.get(0)?,
            title: row.get(1)?,
            created_at: ts(row.get(2)?),
            updated_at: ts(row.get(3)?),
            last_message: row.get(4)?,
        })
    })?;
    rows.collect()
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct MessageRow {
    pub id: String,
    pub conversation_id: String,
    pub run_id: Option<String>,
    pub role: String,
    pub content: String,
    pub created_at: i64,
    pub metadata: Value,
}

impl MessageRow {
    pub fn new(
        id: impl Into<String>,
        conversation_id: &str,
        run_id: Option<&str>,
        role: &str,
        content: impl Into<String>,
        metadata: impl Serialize,
    ) -> Self {
        Self {
            id: id.into(),
            conversation_id: conversation_id.to_string(),
            run_id: run_id.map(str::to_string),
            role: role.to_string(),
            content: content.into(),
            created_at: now_ms(),
            metadata: serde_json::to_value(metadata).unwrap_or(Value::Null),
        }
    }

    /// A message Nandi added to keep a run going (`post_turn`'s
    /// `continue_with`, e.g. verify-on-stop). The model reads it as a user
    /// message, but the user didn't write it, so chats don't show it.
    pub fn is_continuation(&self) -> bool {
        self.role == role::USER && self.metadata["continuation"] == Value::Bool(true)
    }

    /// The UI shape of this row.
    pub fn to_chat(&self) -> ChatMessage {
        let id = self.id.clone();
        let run_id = self.run_id.clone();
        let created_at = ts(self.created_at);
        let meta = self.metadata.clone();
        match self.role.as_str() {
            role::USER => ChatMessage::User {
                id,
                run_id,
                created_at,
                content: self.content.clone(),
                attachments: serde_json::from_value::<UserMeta>(meta)
                    .unwrap_or_default()
                    .attachments,
            },
            role::TOOL_CALL => {
                let meta: ToolCallMeta =
                    serde_json::from_value(meta).unwrap_or_else(|_| ToolCallMeta {
                        tool: "unknown".into(),
                        args: Value::Null,
                        status: ToolCallStatus::Done,
                        result: None,
                        is_error: true,
                    });
                ChatMessage::ToolCall {
                    id,
                    run_id,
                    created_at,
                    tool: meta.tool,
                    args: meta.args,
                    status: meta.status,
                    result: meta.result,
                    is_error: meta.is_error,
                }
            }
            role::APPROVAL => {
                let meta: ApprovalMeta =
                    serde_json::from_value(meta).unwrap_or_else(|_| ApprovalMeta {
                        tool: "unknown".into(),
                        args: Value::Null,
                        status: ApprovalStatus::Expired,
                        expires_at: None,
                    });
                ChatMessage::Approval {
                    id,
                    run_id,
                    created_at,
                    tool: meta.tool,
                    args: meta.args,
                    status: meta.status,
                    expires_at: meta.expires_at.map(ts),
                }
            }
            role::ERROR => ChatMessage::Error {
                id,
                run_id,
                created_at,
                content: self.content.clone(),
            },
            _ => ChatMessage::Agent {
                id,
                run_id,
                created_at,
                content: self.content.clone(),
            },
        }
    }
}

const MESSAGE_COLUMNS: &str = "id, conversation_id, run_id, role, content, created_at, metadata";

fn message_row(row: &Row) -> rusqlite::Result<MessageRow> {
    Ok(MessageRow {
        id: row.get(0)?,
        conversation_id: row.get(1)?,
        run_id: row.get(2)?,
        role: row.get(3)?,
        content: row.get(4)?,
        created_at: row.get(5)?,
        metadata: json_col(row, 6)?,
    })
}

pub fn insert_message(conn: &Connection, message: &MessageRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO messages (id, conversation_id, run_id, role, content, created_at, metadata)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            message.id,
            message.conversation_id,
            message.run_id,
            message.role,
            message.content,
            message.created_at,
            message.metadata.to_string(),
        ],
    )?;
    Ok(())
}

pub fn get_message(conn: &Connection, id: &str) -> rusqlite::Result<Option<MessageRow>> {
    conn.query_row(
        &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
        [id],
        message_row,
    )
    .optional()
}

/// Appends `text` to a message. Returns the offset it was appended at, in
/// UTF-16 code units (JavaScript string length), for idempotent deltas.
pub fn append_message_content(conn: &Connection, id: &str, text: &str) -> rusqlite::Result<usize> {
    let current: String =
        conn.query_row("SELECT content FROM messages WHERE id = ?1", [id], |row| {
            row.get(0)
        })?;
    conn.execute(
        "UPDATE messages SET content = content || ?2 WHERE id = ?1",
        params![id, text],
    )?;
    Ok(current.encode_utf16().count())
}

pub fn update_message_metadata(
    conn: &Connection,
    id: &str,
    metadata: impl Serialize,
) -> rusqlite::Result<()> {
    let metadata = serde_json::to_value(metadata).unwrap_or(Value::Null);
    conn.execute(
        "UPDATE messages SET metadata = ?2 WHERE id = ?1",
        params![id, metadata.to_string()],
    )?;
    Ok(())
}

/// Metadata key on a message that compaction copied into a child run
/// (the original message's ID)
pub const COMPACTION_COPY_KEY: &str = "compactionCopyOf";
/// Metadata key on a child run's synthetic summary message (the parent
/// run's ID)
pub const COMPACTION_SUMMARY_KEY: &str = "compactionSummaryOf";

/// The latest `limit` messages of a conversation, oldest first. Messages
/// compaction wrote into a child run (copies and the summary) are left
/// out: the conversation already has the originals.
pub fn list_messages(
    conn: &Connection,
    conversation_id: &str,
    limit: u32,
) -> rusqlite::Result<Vec<MessageRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT * FROM (
            SELECT {MESSAGE_COLUMNS}, rowid AS seq FROM messages
             WHERE conversation_id = ?1
               AND metadata NOT LIKE '%\"{COMPACTION_COPY_KEY}\"%'
               AND metadata NOT LIKE '%\"{COMPACTION_SUMMARY_KEY}\"%'
            ORDER BY created_at DESC, rowid DESC LIMIT ?2
         ) ORDER BY created_at ASC, seq ASC"
    ))?;
    let rows = stmt.query_map(params![conversation_id, limit], message_row)?;
    rows.collect()
}

pub fn list_run_messages(conn: &Connection, run_id: &str) -> rusqlite::Result<Vec<MessageRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {MESSAGE_COLUMNS} FROM messages WHERE run_id = ?1 ORDER BY created_at, rowid"
    ))?;
    let rows = stmt.query_map([run_id], message_row)?;
    rows.collect()
}

// ---------------------------------------------------------------------------
// Runs
// ---------------------------------------------------------------------------

const RUN_COLUMNS: &str =
    "id, agent_id, conversation_id, status, input, summary, started_at, ended_at,
     input_tokens, output_tokens, cost_usd, model,
     source_type, source_id, source_label, priority, queued_at,
     parent_run_id, compaction_summary";

fn run_row(row: &Row) -> rusqlite::Result<RunSummary> {
    Ok(RunSummary {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        conversation_id: row.get(2)?,
        status: RunStatus::parse(&row.get::<_, String>(3)?),
        input: row.get(4)?,
        summary: row.get(5)?,
        started_at: ts(row.get(6)?),
        ended_at: row.get::<_, Option<i64>>(7)?.map(ts),
        input_tokens: row.get(8)?,
        output_tokens: row.get(9)?,
        cost_usd: row.get(10)?,
        model: row.get(11)?,
        source_type: SourceType::parse(&row.get::<_, String>(12)?),
        source_id: row.get(13)?,
        source_label: row.get(14)?,
        priority: row.get(15)?,
        queued_at: row.get::<_, Option<i64>>(16)?.map(ts),
        parent_run_id: row.get(17)?,
        compaction_summary: row.get(18)?,
    })
}

pub fn insert_run(conn: &Connection, run: &RunSummary) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO runs (id, agent_id, conversation_id, started_at, ended_at, status, input, summary,
                           source_type, source_id, source_label, priority, queued_at,
                           parent_run_id, compaction_summary)
         VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, NULL, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            run.id,
            run.agent_id,
            run.conversation_id,
            run.started_at as i64,
            run.status.as_str(),
            run.input,
            run.source_type.as_str(),
            run.source_id,
            run.source_label,
            run.priority,
            run.queued_at.map(|t| t as i64),
            run.parent_run_id,
            run.compaction_summary,
        ],
    )?;
    Ok(())
}

/// The runs continuing `parent_run_id` after compaction, oldest first.
#[allow(dead_code)] // The UI builds lineage from `parent_run_id` on run lists
pub fn child_runs(conn: &Connection, parent_run_id: &str) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM runs WHERE parent_run_id = ?1 ORDER BY started_at, id"
    ))?;
    let rows = stmt.query_map([parent_run_id], run_row)?;
    rows.collect()
}

/// Ends `parent` as `compacted` and starts `child` (which must name it in
/// `parent_run_id`) with `messages`, in one transaction. Returns both runs
/// as updated.
pub fn start_compacted_child(
    conn: &mut Connection,
    child: &RunSummary,
    messages: &[MessageRow],
) -> rusqlite::Result<(RunSummary, RunSummary)> {
    let parent_id = child.parent_run_id.clone().unwrap_or_default();
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let parent = set_run_status(&tx, &parent_id, RunStatus::Compacted, None)?
        .ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    insert_run(&tx, child)?;
    for message in messages {
        insert_message(&tx, message)?;
    }
    let child = get_run(&tx, &child.id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    tx.commit()?;
    Ok((parent, child))
}

pub fn get_run(conn: &Connection, id: &str) -> rusqlite::Result<Option<RunSummary>> {
    conn.query_row(
        &format!("SELECT {RUN_COLUMNS} FROM runs WHERE id = ?1"),
        [id],
        run_row,
    )
    .optional()
}

/// Updates a run's status; terminal statuses also set `ended_at`. Returns
/// the updated run.
pub fn set_run_status(
    conn: &Connection,
    id: &str,
    status: RunStatus,
    summary: Option<&str>,
) -> rusqlite::Result<Option<RunSummary>> {
    let ended_at = status.is_finished().then(now_ms);
    conn.execute(
        "UPDATE runs SET status = ?2, ended_at = ?3, summary = COALESCE(?4, summary) WHERE id = ?1",
        params![id, status.as_str(), ended_at, summary],
    )?;
    get_run(conn, id)
}

/// Adds one model call's usage to the run's totals and records the model
/// (the latest call's). The cost stays NULL until a call with known
/// pricing is recorded.
pub fn add_run_usage(
    conn: &Connection,
    id: &str,
    model: &str,
    input_tokens: u32,
    output_tokens: u32,
    cost_usd: Option<f64>,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE runs SET input_tokens = input_tokens + ?2,
                         output_tokens = output_tokens + ?3,
                         cost_usd = CASE WHEN ?4 IS NULL THEN cost_usd
                                         ELSE COALESCE(cost_usd, 0) + ?4 END,
                         model = ?5
          WHERE id = ?1",
        params![id, input_tokens, output_tokens, cost_usd, model],
    )?;
    Ok(())
}

/// The conversation's running or approval-blocked run, if any.
pub fn active_run(
    conn: &Connection,
    conversation_id: &str,
) -> rusqlite::Result<Option<RunSummary>> {
    conn.query_row(
        &format!(
            "SELECT {RUN_COLUMNS} FROM runs
              WHERE conversation_id = ?1 AND status IN ('running', 'awaiting_approval')
              ORDER BY started_at DESC LIMIT 1"
        ),
        [conversation_id],
        run_row,
    )
    .optional()
}

/// The agent's current turn: its running or approval-blocked run.
pub fn active_agent_run(conn: &Connection, agent_id: &str) -> rusqlite::Result<Option<RunSummary>> {
    conn.query_row(
        &format!(
            "SELECT {RUN_COLUMNS} FROM runs
              WHERE agent_id = ?1 AND status IN ('running', 'awaiting_approval')
              ORDER BY started_at DESC LIMIT 1"
        ),
        [agent_id],
        run_row,
    )
    .optional()
}

/// Every running or approval-blocked run.
/// An agent's running (or approval-waiting) run in any of its
/// conversations: its chat, or a delegated task's
pub fn active_run_of_agent(
    conn: &Connection,
    agent_id: &str,
) -> rusqlite::Result<Option<RunSummary>> {
    conn.query_row(
        &format!(
            "SELECT {RUN_COLUMNS} FROM runs
              WHERE agent_id = ?1 AND status IN ('running', 'awaiting_approval')
              ORDER BY started_at DESC LIMIT 1"
        ),
        [agent_id],
        run_row,
    )
    .optional()
}

pub fn active_runs(conn: &Connection) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM runs WHERE status IN ('running', 'awaiting_approval')"
    ))?;
    let rows = stmt.query_map([], run_row)?;
    rows.collect()
}

/// Runs newest first, optionally for one agent. Studio runs are excluded
/// when no agent is given.
pub fn list_runs(
    conn: &Connection,
    agent_id: Option<&str>,
    limit: u32,
) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM runs
          WHERE (?1 IS NULL AND agent_id != ?3) OR agent_id = ?1
          ORDER BY started_at DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![agent_id, limit, STUDIO_AGENT_ID], run_row)?;
    rows.collect()
}

pub fn runs_since(conn: &Connection, since_ms: i64) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM runs WHERE started_at >= ?1 AND agent_id != ?2
          ORDER BY started_at DESC"
    ))?;
    let rows = stmt.query_map(params![since_ms, STUDIO_AGENT_ID], run_row)?;
    rows.collect()
}

/// Runs left `running` by a previous process can't continue: their
/// in-memory step died with it. Marks them failed and returns them.
pub fn fail_interrupted_runs(conn: &Connection) -> rusqlite::Result<Vec<RunSummary>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {RUN_COLUMNS} FROM runs WHERE status = 'running'"
    ))?;
    let runs: Vec<RunSummary> = stmt.query_map([], run_row)?.collect::<Result<_, _>>()?;
    for run in &runs {
        set_run_status(
            conn,
            &run.id,
            RunStatus::Failed,
            Some("Interrupted: the app was closed while this run was in progress."),
        )?;
    }
    Ok(runs)
}

// ---------------------------------------------------------------------------
// Agent queue
// ---------------------------------------------------------------------------

/// A queued request: everything needed to start its run later.
#[derive(Debug, Clone, PartialEq)]
pub struct QueueRow {
    pub id: String,
    pub agent_id: String,
    pub run_id: String,
    pub source_type: SourceType,
    pub source_id: Option<String>,
    pub source_label: Option<String>,
    pub priority: i32,
    pub message: String,
    pub message_id: String,
    pub attachments: Vec<AttachmentInfo>,
    pub created_at: i64,
}

impl QueueRow {
    fn to_entry(&self, position: u32) -> QueueEntry {
        QueueEntry {
            id: self.id.clone(),
            agent_id: self.agent_id.clone(),
            run_id: self.run_id.clone(),
            source_type: self.source_type,
            source_id: self.source_id.clone(),
            source_label: self.source_label.clone(),
            priority: self.priority,
            message: self.message.clone(),
            attachments: self.attachments.clone(),
            created_at: ts(self.created_at),
            position,
        }
    }
}

const QUEUE_COLUMNS: &str = "id, agent_id, run_id, source_type, source_id, source_label, priority,
     message, message_id, attachments, created_at";
/// Next to run first. `rowid` breaks ties within the same millisecond.
const QUEUE_ORDER: &str = "priority ASC, created_at ASC, rowid ASC";

fn queue_row(row: &Row) -> rusqlite::Result<QueueRow> {
    let attachments: Option<String> = row.get(9)?;
    Ok(QueueRow {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        run_id: row.get(2)?,
        source_type: SourceType::parse(&row.get::<_, String>(3)?),
        source_id: row.get(4)?,
        source_label: row.get(5)?,
        priority: row.get(6)?,
        message: row.get(7)?,
        message_id: row.get(8)?,
        attachments: attachments
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default(),
        created_at: row.get(10)?,
    })
}

pub fn insert_queue_row(conn: &Connection, entry: &QueueRow) -> rusqlite::Result<()> {
    let attachments = (!entry.attachments.is_empty())
        .then(|| serde_json::to_string(&entry.attachments).unwrap_or_default());
    conn.execute(
        &format!(
            "INSERT INTO agent_queue ({QUEUE_COLUMNS})
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
        ),
        params![
            entry.id,
            entry.agent_id,
            entry.run_id,
            entry.source_type.as_str(),
            entry.source_id,
            entry.source_label,
            entry.priority,
            entry.message,
            entry.message_id,
            attachments,
            entry.created_at,
        ],
    )?;
    renumber_queue(conn, &entry.agent_id)
}

fn queue_rows(conn: &Connection, agent_id: &str) -> rusqlite::Result<Vec<QueueRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {QUEUE_COLUMNS} FROM agent_queue WHERE agent_id = ?1 ORDER BY {QUEUE_ORDER}"
    ))?;
    let rows = stmt.query_map([agent_id], queue_row)?;
    rows.collect()
}

/// The agent's queue, next to run first.
pub fn list_queue(conn: &Connection, agent_id: &str) -> rusqlite::Result<Vec<QueueEntry>> {
    Ok(queue_rows(conn, agent_id)?
        .iter()
        .enumerate()
        .map(|(position, row)| row.to_entry(position as u32))
        .collect())
}

pub fn queue_is_empty(conn: &Connection, agent_id: &str) -> rusqlite::Result<bool> {
    let mut stmt = conn.prepare("SELECT 1 FROM agent_queue WHERE agent_id = ?1 LIMIT 1")?;
    Ok(!stmt.exists([agent_id])?)
}

pub fn queued_run(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<QueueRow>> {
    conn.query_row(
        &format!("SELECT {QUEUE_COLUMNS} FROM agent_queue WHERE run_id = ?1"),
        [run_id],
        queue_row,
    )
    .optional()
}

/// Removes and returns the agent's next request: lowest priority value,
/// then oldest.
pub fn pop_queue(conn: &Connection, agent_id: &str) -> rusqlite::Result<Option<QueueRow>> {
    let next = conn
        .query_row(
            &format!(
                "SELECT {QUEUE_COLUMNS} FROM agent_queue WHERE agent_id = ?1
                  ORDER BY {QUEUE_ORDER} LIMIT 1"
            ),
            [agent_id],
            queue_row,
        )
        .optional()?;
    if let Some(row) = &next {
        delete_queue_row(conn, row)?;
    }
    Ok(next)
}

/// Removes a queued request. Returns it, or `None` if it isn't queued.
pub fn remove_queued(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<QueueRow>> {
    let row = queued_run(conn, run_id)?;
    if let Some(row) = &row {
        delete_queue_row(conn, row)?;
    }
    Ok(row)
}

fn delete_queue_row(conn: &Connection, row: &QueueRow) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM agent_queue WHERE id = ?1", [&row.id])?;
    conn.execute(
        "UPDATE runs SET queue_position = NULL WHERE id = ?1",
        [&row.run_id],
    )?;
    renumber_queue(conn, &row.agent_id)
}

/// Changes a queued request's priority. Returns whether it was queued.
pub fn set_queue_priority(
    conn: &Connection,
    run_id: &str,
    priority: i32,
) -> rusqlite::Result<bool> {
    let Some(row) = queued_run(conn, run_id)? else {
        return Ok(false);
    };
    conn.execute(
        "UPDATE agent_queue SET priority = ?2 WHERE id = ?1",
        params![row.id, priority],
    )?;
    conn.execute(
        "UPDATE runs SET priority = ?2 WHERE id = ?1",
        params![run_id, priority],
    )?;
    renumber_queue(conn, &row.agent_id)?;
    Ok(true)
}

/// Mirrors each queued request's position onto its run.
fn renumber_queue(conn: &Connection, agent_id: &str) -> rusqlite::Result<()> {
    for (position, row) in queue_rows(conn, agent_id)?.iter().enumerate() {
        conn.execute(
            "UPDATE runs SET queue_position = ?2 WHERE id = ?1",
            params![row.run_id, position as i64],
        )?;
    }
    Ok(())
}

/// Marks a queued run as started now.
pub fn start_queued_run(conn: &Connection, run_id: &str) -> rusqlite::Result<Option<RunSummary>> {
    conn.execute(
        "UPDATE runs SET status = 'running', started_at = ?2, queue_position = NULL WHERE id = ?1",
        params![run_id, now_ms()],
    )?;
    get_run(conn, run_id)
}

// ---------------------------------------------------------------------------
// Run events
// ---------------------------------------------------------------------------

pub fn insert_run_event(
    conn: &Connection,
    run_id: &str,
    kind: &str,
    payload: &Value,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO run_events (run_id, kind, payload, created_at) VALUES (?1, ?2, ?3, ?4)",
        params![run_id, kind, payload.to_string(), now_ms()],
    )?;
    Ok(())
}

/// The latest events across an agent's runs, oldest first.
pub fn list_agent_events(
    conn: &Connection,
    agent_id: &str,
    limit: u32,
) -> rusqlite::Result<Vec<RunEventRecord>> {
    let mut stmt = conn.prepare(
        "SELECT * FROM (
            SELECT e.id, e.run_id, e.kind, e.payload, e.created_at
              FROM run_events e JOIN runs r ON r.id = e.run_id
             WHERE r.agent_id = ?1
             ORDER BY e.id DESC LIMIT ?2
         ) ORDER BY id ASC",
    )?;
    let rows = stmt.query_map(params![agent_id, limit], |row| {
        Ok(RunEventRecord {
            id: row.get::<_, i64>(0)? as f64,
            run_id: row.get(1)?,
            kind: row.get(2)?,
            payload: json_col(row, 3)?,
            created_at: ts(row.get(4)?),
        })
    })?;
    rows.collect()
}

// ---------------------------------------------------------------------------
// Approvals
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalRow {
    pub id: String,
    pub agent_id: String,
    pub run_id: String,
    pub message_id: String,
    pub tool_call_id: String,
    pub tool: String,
    pub args: Value,
    pub status: ApprovalStatus,
    pub created_at: i64,
    /// Epoch ms; `None` never expires (escalated clarifications have their
    /// own deadline)
    pub expires_at: Option<i64>,
    /// Who approved or rejected it (`set_approval_decider`)
    pub decided_by: Option<String>,
}

const APPROVAL_COLUMNS: &str = "id, agent_id, run_id, message_id, tool_call_id, tool, args, \
     status, created_at, expires_at, decided_by";

fn approval_row(row: &Row) -> rusqlite::Result<ApprovalRow> {
    Ok(ApprovalRow {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        run_id: row.get(2)?,
        message_id: row.get(3)?,
        tool_call_id: row.get(4)?,
        tool: row.get(5)?,
        args: json_col(row, 6)?,
        status: ApprovalStatus::parse(&row.get::<_, String>(7)?),
        created_at: row.get(8)?,
        expires_at: row.get(9)?,
        decided_by: row.get(10)?,
    })
}

/// Records who decided `id`, before the decision closes it
pub fn set_approval_decider(conn: &Connection, id: &str, who: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE approvals SET decided_by = ?2 WHERE id = ?1",
        params![id, who],
    )?;
    Ok(())
}

pub fn insert_approval(conn: &Connection, approval: &ApprovalRow) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO approvals (id, agent_id, run_id, message_id, tool_call_id, tool, args, status, created_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            approval.id,
            approval.agent_id,
            approval.run_id,
            approval.message_id,
            approval.tool_call_id,
            approval.tool,
            approval.args.to_string(),
            approval.status.as_str(),
            approval.created_at,
            approval.expires_at,
        ],
    )?;
    Ok(())
}

/// Pending approvals whose deadline has passed, oldest first
pub fn overdue_approvals(conn: &Connection, now_ms: i64) -> rusqlite::Result<Vec<ApprovalRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {APPROVAL_COLUMNS} FROM approvals
          WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at <= ?1
          ORDER BY created_at"
    ))?;
    let rows = stmt.query_map([now_ms], approval_row)?;
    rows.collect()
}

pub fn get_approval(conn: &Connection, id: &str) -> rusqlite::Result<Option<ApprovalRow>> {
    conn.query_row(
        &format!("SELECT {APPROVAL_COLUMNS} FROM approvals WHERE id = ?1"),
        [id],
        approval_row,
    )
    .optional()
}

pub fn approval_for_call(
    conn: &Connection,
    tool_call_id: &str,
) -> rusqlite::Result<Option<ApprovalRow>> {
    conn.query_row(
        &format!("SELECT {APPROVAL_COLUMNS} FROM approvals WHERE tool_call_id = ?1"),
        [tool_call_id],
        approval_row,
    )
    .optional()
}

/// Sets an approval's status and mirrors it onto its chat message.
pub fn set_approval_status(
    conn: &Connection,
    approval: &ApprovalRow,
    status: ApprovalStatus,
) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE approvals SET status = ?2, resolved_at = ?3 WHERE id = ?1",
        params![approval.id, status.as_str(), now_ms()],
    )?;
    update_message_metadata(
        conn,
        &approval.message_id,
        ApprovalMeta {
            tool: approval.tool.clone(),
            args: approval.args.clone(),
            status,
            expires_at: approval.expires_at,
        },
    )
}

/// Pending approvals, oldest first, optionally for one agent.
pub fn pending_approvals(
    conn: &Connection,
    agent_id: Option<&str>,
) -> rusqlite::Result<Vec<PendingApproval>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.agent_id, a.run_id, r.conversation_id, a.tool, a.args, a.created_at, a.expires_at
           FROM approvals a JOIN runs r ON r.id = a.run_id
          WHERE a.status = 'pending' AND (?1 IS NULL OR a.agent_id = ?1)
          ORDER BY a.created_at",
    )?;
    let rows = stmt.query_map(params![agent_id], |row| {
        Ok(PendingApproval {
            id: row.get(0)?,
            agent_id: row.get(1)?,
            run_id: row.get(2)?,
            conversation_id: row.get(3)?,
            tool: row.get(4)?,
            args: json_col(row, 5)?,
            created_at: ts(row.get(6)?),
            expires_at: row.get::<_, Option<i64>>(7)?.map(ts),
        })
    })?;
    rows.collect()
}

/// Every approval in a conversation, by the tool call it gates.
pub fn approvals_by_call(
    conn: &Connection,
    conversation_id: &str,
) -> rusqlite::Result<std::collections::HashMap<String, ApprovalStatus>> {
    let mut stmt = conn.prepare(
        "SELECT a.tool_call_id, a.status FROM approvals a JOIN runs r ON r.id = a.run_id
          WHERE r.conversation_id = ?1",
    )?;
    let rows = stmt.query_map([conversation_id], |row| {
        Ok((
            row.get::<_, String>(0)?,
            ApprovalStatus::parse(&row.get::<_, String>(1)?),
        ))
    })?;
    rows.collect()
}

pub fn pending_approvals_for_run(
    conn: &Connection,
    run_id: &str,
) -> rusqlite::Result<Vec<ApprovalRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {APPROVAL_COLUMNS} FROM approvals WHERE run_id = ?1 AND status = 'pending'"
    ))?;
    let rows = stmt.query_map([run_id], approval_row)?;
    rows.collect()
}

// ---------------------------------------------------------------------------
// Settings, agent state, workflows
// ---------------------------------------------------------------------------

/// Which turn engine runs agents: the LLM (default) or the canned script.
pub const ENGINE_KEY: &str = "engine";

pub fn uses_canned_engine(conn: &Connection) -> rusqlite::Result<bool> {
    Ok(
        get_setting(conn, ENGINE_KEY)?.and_then(|v| v.as_str().map(|s| s == "canned"))
            == Some(true),
    )
}

/// A machine-local runtime setting (`local_settings`).
pub fn get_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<Value>> {
    conn.query_row(
        "SELECT value FROM local_settings WHERE key = ?1",
        [key],
        |row| json_col(row, 0),
    )
    .optional()
}

/// Writes a machine-local runtime setting (`local_settings`).
pub fn set_setting(conn: &Connection, key: &str, value: &Value) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO local_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value.to_string()],
    )?;
    Ok(())
}

/// Removes a machine-local runtime setting.
pub fn delete_setting(conn: &Connection, key: &str) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM local_settings WHERE key = ?1", [key])?;
    Ok(())
}

pub fn set_agent_state(
    conn: &Connection,
    agent_id: &str,
    status: &str,
    error: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO agent_state (agent_id, status, error, updated_at) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(agent_id) DO UPDATE SET status = excluded.status, error = excluded.error,
                                             updated_at = excluded.updated_at",
        params![agent_id, status, error, now_ms()],
    )?;
    Ok(())
}

/// Agents whose orchestrator is in the `error` state, with the error.
pub fn agents_in_error(conn: &Connection) -> rusqlite::Result<Vec<(String, Option<String>)>> {
    let mut stmt =
        conn.prepare("SELECT agent_id, error FROM agent_state WHERE status = 'error'")?;
    let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect()
}

pub fn list_workflows(conn: &Connection) -> rusqlite::Result<Vec<WorkflowSummary>> {
    let mut stmt =
        conn.prepare("SELECT id, name, definition, created_at FROM workflows ORDER BY name")?;
    let rows = stmt.query_map([], |row| {
        Ok(WorkflowSummary {
            id: row.get(0)?,
            name: row.get(1)?,
            definition: json_col(row, 2)?,
            created_at: ts(row.get(3)?),
        })
    })?;
    rows.collect()
}

// ---------------------------------------------------------------------------
// Agent memory (wave final, Track C): a rebuildable cache of memory.md
// ---------------------------------------------------------------------------

/// Replaces the cached entries of `agent_id` with `entries`, in file order.
pub fn replace_agent_memory(
    conn: &mut Connection,
    agent_id: &str,
    entries: &[crate::services::agent_memory::MemoryEntry],
) -> rusqlite::Result<()> {
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute("DELETE FROM agent_memory WHERE agent_id = ?1", [agent_id])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO agent_memory (agent_id, idx, content, created_at, source_run)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )?;
        for (idx, entry) in entries.iter().enumerate() {
            stmt.execute(params![
                agent_id,
                idx as i64,
                entry.content,
                entry.created_at as i64,
                entry.source_run,
            ])?;
        }
    }
    tx.commit()
}

/// The cached entries of `agent_id`, in file order.
#[cfg(test)]
pub fn list_agent_memory(
    conn: &Connection,
    agent_id: &str,
) -> rusqlite::Result<Vec<crate::services::agent_memory::MemoryEntry>> {
    let mut stmt = conn.prepare(
        "SELECT content, created_at, source_run FROM agent_memory
          WHERE agent_id = ?1 ORDER BY idx",
    )?;
    let rows = stmt.query_map([agent_id], |row| {
        Ok(crate::services::agent_memory::MemoryEntry {
            content: row.get(0)?,
            created_at: ts(row.get(1)?),
            source_run: row.get(2)?,
        })
    })?;
    rows.collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub fn start_run(conn: &Connection, agent_id: &str, run_id: &str) -> String {
        let conversation_id = ensure_agent_conversation(conn, agent_id).unwrap();
        insert_run(
            conn,
            &RunSummary {
                id: run_id.into(),
                agent_id: agent_id.into(),
                conversation_id: conversation_id.clone(),
                status: RunStatus::Running,
                input: "hi".into(),
                summary: None,
                started_at: ts(now_ms()),
                ended_at: None,
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: None,
                model: None,
                source_type: SourceType::User,
                source_id: None,
                source_label: None,
                priority: 100,
                queued_at: None,
                parent_run_id: None,
                compaction_summary: None,
            },
        )
        .unwrap();
        conversation_id
    }

    #[test]
    fn a_database_from_another_schema_is_refused_with_what_to_do() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION - 1)
            .unwrap();
        let error = init_db(&conn).unwrap_err();
        assert!(error.contains("delete ~/.nandi/local/state.db"), "{error}");
        // A fresh one (version 0) and its own version are fine
        let fresh = Connection::open_in_memory().unwrap();
        init_db(&fresh).unwrap();
        init_db(&fresh).unwrap();
    }

    #[test]
    fn init_is_idempotent_and_seeds_settings() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        init_db(&conn).unwrap();

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);
        assert!(get_setting(&conn, crate::engine::CANNED_SCRIPT_KEY)
            .unwrap()
            .is_some());
    }

    #[test]
    fn appends_content_with_utf16_offsets() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        let conversation = ensure_agent_conversation(&conn, "a").unwrap();
        insert_message(
            &conn,
            &MessageRow::new("m1", &conversation, None, role::ASSISTANT, "", json!({})),
        )
        .unwrap();

        assert_eq!(append_message_content(&conn, "m1", "😀 ").unwrap(), 0);
        // The emoji is 2 UTF-16 units
        assert_eq!(append_message_content(&conn, "m1", "hi").unwrap(), 3);
        assert_eq!(get_message(&conn, "m1").unwrap().unwrap().content, "😀 hi");
    }

    #[test]
    fn lists_latest_messages_in_order() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        let conversation = ensure_agent_conversation(&conn, "a").unwrap();
        for i in 0..5 {
            insert_message(
                &conn,
                &MessageRow::new(
                    format!("m{i}"),
                    &conversation,
                    None,
                    role::USER,
                    format!("{i}"),
                    json!({}),
                ),
            )
            .unwrap();
        }

        let ids: Vec<_> = list_messages(&conn, &conversation, 3)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, ["m2", "m3", "m4"]);
    }

    #[test]
    fn only_nandis_continuations_are_continuations() {
        let added = json!({ "attachments": [], "continuation": true });
        let continuation =
            MessageRow::new("c1", "c", Some("r"), role::USER, "verify", added.clone());
        assert!(continuation.is_continuation());
        let typed = MessageRow::new("u1", "c", Some("r"), role::USER, "hey", json!({}));
        assert!(!typed.is_continuation());
        let reply = MessageRow::new("a1", "c", Some("r"), role::ASSISTANT, "ok", added);
        assert!(!reply.is_continuation(), "only user-role rows");
    }

    #[test]
    fn maps_rows_to_chat_messages() {
        let tool = MessageRow::new(
            "t1",
            "c",
            Some("r"),
            role::TOOL_CALL,
            "",
            ToolCallMeta {
                tool: "shell".into(),
                args: json!({"cmd": "ls"}),
                status: ToolCallStatus::Done,
                result: Some("ok".into()),
                is_error: false,
            },
        );
        let value = serde_json::to_value(tool.to_chat()).unwrap();
        assert_eq!(value["kind"], "toolCall");
        assert_eq!(value["runId"], "r");
        assert_eq!(value["isError"], false);

        let user = MessageRow::new("u1", "c", None, role::USER, "hey", json!({}));
        assert!(
            matches!(user.to_chat(), ChatMessage::User { attachments, .. } if attachments.is_empty())
        );
    }

    #[test]
    fn tracks_active_runs_and_interrupted_runs() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        let conversation = start_run(&conn, "a", "r1");

        assert_eq!(active_run(&conn, &conversation).unwrap().unwrap().id, "r1");
        set_run_status(&conn, "r1", RunStatus::AwaitingApproval, None).unwrap();
        assert!(
            fail_interrupted_runs(&conn).unwrap().is_empty(),
            "awaiting runs survive restarts"
        );

        set_run_status(&conn, "r1", RunStatus::Running, None).unwrap();
        let failed = fail_interrupted_runs(&conn).unwrap();
        assert_eq!(failed.len(), 1);
        let run = get_run(&conn, "r1").unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Failed);
        assert!(run.ended_at.is_some());
        assert!(active_run(&conn, &conversation).unwrap().is_none());
    }

    fn run_usage(conn: &Connection, id: &str) -> (u32, u32, Option<f64>, Option<String>) {
        let run = get_run(conn, id).unwrap().unwrap();
        (run.input_tokens, run.output_tokens, run.cost_usd, run.model)
    }

    #[test]
    fn sums_usage_across_model_calls() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        start_run(&conn, "a", "r1");
        assert_eq!(run_usage(&conn, "r1"), (0, 0, None, None));

        let model = "deepseek:deepseek-flash";
        add_run_usage(&conn, "r1", model, 100, 20, None).unwrap();
        assert_eq!(
            run_usage(&conn, "r1"),
            (100, 20, None, Some(model.to_string()))
        );
        add_run_usage(&conn, "r1", model, 50, 5, Some(0.25)).unwrap();
        add_run_usage(&conn, "r1", model, 10, 1, Some(0.5)).unwrap();
        assert_eq!(
            run_usage(&conn, "r1"),
            (160, 26, Some(0.75), Some(model.to_string()))
        );
    }

    fn enqueue(conn: &Connection, agent_id: &str, run_id: &str, priority: i32, created_at: i64) {
        let conversation_id = ensure_agent_conversation(conn, agent_id).unwrap();
        insert_run(
            conn,
            &RunSummary {
                id: run_id.into(),
                agent_id: agent_id.into(),
                conversation_id,
                status: RunStatus::Queued,
                input: run_id.into(),
                summary: None,
                started_at: ts(created_at),
                ended_at: None,
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: None,
                model: None,
                source_type: SourceType::User,
                source_id: None,
                source_label: None,
                priority,
                queued_at: Some(ts(created_at)),
                parent_run_id: None,
                compaction_summary: None,
            },
        )
        .unwrap();
        insert_queue_row(
            conn,
            &QueueRow {
                id: format!("q-{run_id}"),
                agent_id: agent_id.into(),
                run_id: run_id.into(),
                source_type: SourceType::User,
                source_id: None,
                source_label: None,
                priority,
                message: run_id.into(),
                message_id: format!("m-{run_id}"),
                attachments: vec![],
                created_at,
            },
        )
        .unwrap();
    }

    fn queue_ids(conn: &Connection, agent_id: &str) -> Vec<String> {
        list_queue(conn, agent_id)
            .unwrap()
            .into_iter()
            .map(|e| e.run_id)
            .collect()
    }

    fn queue_position(conn: &Connection, run_id: &str) -> Option<i64> {
        conn.query_row(
            "SELECT queue_position FROM runs WHERE id = ?1",
            [run_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn queue_orders_by_priority_then_fifo() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        enqueue(&conn, "a", "bg-old", 50, 1);
        enqueue(&conn, "a", "user-old", 10, 2);
        enqueue(&conn, "a", "bg-new", 50, 3);
        enqueue(&conn, "a", "user-new", 10, 4);
        // Same priority and millisecond: insertion order
        enqueue(&conn, "a", "user-tie", 10, 4);
        enqueue(&conn, "other", "elsewhere", 1, 0);

        assert_eq!(
            queue_ids(&conn, "a"),
            ["user-old", "user-new", "user-tie", "bg-old", "bg-new"]
        );
        let entries = list_queue(&conn, "a").unwrap();
        assert_eq!(
            entries.iter().map(|e| e.position).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        assert_eq!(queue_position(&conn, "bg-old"), Some(3));

        let popped: Vec<String> = std::iter::from_fn(|| pop_queue(&conn, "a").unwrap())
            .map(|row| row.run_id)
            .collect();
        assert_eq!(
            popped,
            ["user-old", "user-new", "user-tie", "bg-old", "bg-new"]
        );
        assert!(queue_is_empty(&conn, "a").unwrap());
        assert_eq!(queue_position(&conn, "bg-new"), None);
        assert_eq!(queue_ids(&conn, "other"), ["elsewhere"]);
    }

    #[test]
    fn removes_and_reprioritizes_queued_runs() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        enqueue(&conn, "a", "r1", 50, 1);
        enqueue(&conn, "a", "r2", 50, 2);
        enqueue(&conn, "a", "r3", 50, 3);

        let removed = remove_queued(&conn, "r2").unwrap().unwrap();
        assert_eq!(removed.message_id, "m-r2");
        assert!(remove_queued(&conn, "r2").unwrap().is_none());
        assert_eq!(queue_ids(&conn, "a"), ["r1", "r3"]);
        assert_eq!(queue_position(&conn, "r3"), Some(1));

        assert!(set_queue_priority(&conn, "r3", 5).unwrap());
        assert_eq!(queue_ids(&conn, "a"), ["r3", "r1"]);
        assert_eq!(get_run(&conn, "r3").unwrap().unwrap().priority, 5);
        assert_eq!(queue_position(&conn, "r1"), Some(1));
        assert!(!set_queue_priority(&conn, "missing", 1).unwrap());
    }

    #[test]
    fn queued_runs_are_neither_active_nor_ended() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        enqueue(&conn, "a", "r1", 10, 1);
        let run = get_run(&conn, "r1").unwrap().unwrap();
        assert_eq!(run.status, RunStatus::Queued);
        assert!(run.ended_at.is_none());
        assert!(active_agent_run(&conn, "a").unwrap().is_none());
        assert!(fail_interrupted_runs(&conn).unwrap().is_empty());

        let started = start_queued_run(&conn, "r1").unwrap().unwrap();
        assert_eq!(started.status, RunStatus::Running);
        assert!(started.started_at >= run.started_at);
        assert_eq!(started.queued_at, Some(ts(1)));
        assert_eq!(active_agent_run(&conn, "a").unwrap().unwrap().id, "r1");
    }

    #[test]
    fn approvals_mirror_status_onto_their_message() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        let conversation = start_run(&conn, "a", "r1");
        insert_message(
            &conn,
            &MessageRow::new(
                "ap1",
                &conversation,
                Some("r1"),
                role::APPROVAL,
                "",
                ApprovalMeta {
                    tool: "shell".into(),
                    args: json!({}),
                    status: ApprovalStatus::Pending,
                    expires_at: Some(5_000),
                },
            ),
        )
        .unwrap();
        let approval = ApprovalRow {
            id: "ap1".into(),
            agent_id: "a".into(),
            run_id: "r1".into(),
            message_id: "ap1".into(),
            tool_call_id: "call1".into(),
            tool: "shell".into(),
            args: json!({}),
            status: ApprovalStatus::Pending,
            created_at: now_ms(),
            expires_at: Some(5_000),
            decided_by: None,
        };
        insert_approval(&conn, &approval).unwrap();

        let pending = &pending_approvals(&conn, Some("a")).unwrap()[0];
        assert_eq!(pending.conversation_id, conversation);
        assert_eq!(pending.expires_at, Some(5_000.0));
        assert_eq!(
            approval_for_call(&conn, "call1").unwrap().unwrap().id,
            "ap1"
        );
        // Overdue once its deadline has passed
        assert!(overdue_approvals(&conn, 4_999).unwrap().is_empty());
        assert_eq!(overdue_approvals(&conn, 5_000).unwrap()[0].id, "ap1");

        set_approval_decider(&conn, "ap1", "Ada").unwrap();
        assert_eq!(
            get_approval(&conn, "ap1")
                .unwrap()
                .unwrap()
                .decided_by
                .as_deref(),
            Some("Ada")
        );
        set_approval_status(&conn, &approval, ApprovalStatus::Approved).unwrap();
        assert!(pending_approvals(&conn, None).unwrap().is_empty());
        assert!(overdue_approvals(&conn, i64::MAX).unwrap().is_empty());
        let message = get_message(&conn, "ap1").unwrap().unwrap();
        // The deadline stays on the message, for "expired" vs "decided"
        assert!(matches!(
            message.to_chat(),
            ChatMessage::Approval {
                status: ApprovalStatus::Approved,
                expires_at: Some(5_000.0),
                ..
            }
        ));
    }

    #[test]
    fn summarizes_studio_conversations_newest_first() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        insert_studio_conversation(&conn, "s1", "First").unwrap();
        insert_studio_conversation(&conn, "s2", "Second").unwrap();
        let mut reply = MessageRow::new("m", "s1", None, role::ASSISTANT, "latest", json!({}));
        reply.created_at += 10_000;
        insert_message(&conn, &reply).unwrap();
        // Agent conversations aren't Studio conversations
        ensure_agent_conversation(&conn, "a").unwrap();

        let list = list_studio_conversations(&conn).unwrap();
        assert_eq!(
            list.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            ["s1", "s2"]
        );
        assert_eq!(list[0].last_message.as_deref(), Some("latest"));
    }

    #[test]
    fn records_agent_state() {
        let db = Db::in_memory();
        let conn = db.conn().unwrap();
        set_agent_state(&conn, "a", "error", Some("panicked")).unwrap();
        set_agent_state(&conn, "b", "ok", None).unwrap();
        assert_eq!(
            agents_in_error(&conn).unwrap(),
            [("a".to_string(), Some("panicked".to_string()))]
        );
    }
}
