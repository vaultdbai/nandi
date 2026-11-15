-- Migration 006: from Nandi 0.1.0 (version 2) to the schema of
-- 2026-11-15. It skips 3 to 5: those versions were development builds
-- that were never released, and `migrations::upgrade` refuses them.
--
-- Every row a person owns is kept. What goes:
-- - The tables of removed features: git-registry teams and their rosters,
--   subscribed capability registries, and the gateway's circuit breakers
--   (`provider_circuits` does that job)
-- - Three columns nothing reads: `capabilities_projection.registry_origin`,
--   `capability_evidence.approval_required`, `reassigned_tasks.new_task_id`
-- - The rows of the gold copy's projection and the capability index. They
--   are views of `~/.nandi/repo`, rebuilt at the next start from the files
--   (which the gold copy upgrade has moved to Nandi by then).
--
-- The built-in agent `chief-of-staff` is now `nandi`, so every agent ID
-- column says `nandi` where it said `chief-of-staff`, including the
-- conversations of Inbox and intake threads. Where a row for `nandi`
-- already exists in a table keyed by agent, that row is kept and the old
-- one stays under its old ID (UPDATE OR IGNORE).

-- Removed features
DROP TABLE teams;
DROP TABLE team_members;
DROP TABLE roster_cache;
DROP TABLE capability_registries;
DROP TABLE circuit_breakers;

ALTER TABLE capabilities_projection DROP COLUMN registry_origin;
ALTER TABLE capability_evidence DROP COLUMN approval_required;
ALTER TABLE reassigned_tasks DROP COLUMN new_task_id;

-- New tables, word for word as the development builds made them

-- Provider side: results kept until the relay has them (a2a::result_outbox)
CREATE TABLE result_outbox (
    task_id TEXT PRIMARY KEY,
    result  TEXT NOT NULL,
    kept_at INTEGER NOT NULL
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

-- The projection and the capability index: rebuilt from the gold copy at
-- the next start (`projection::needs_rebuild` sees no settings row)
DELETE FROM agents;
DELETE FROM workflows;
DELETE FROM triggers;
DELETE FROM settings;
DELETE FROM connectors;
DELETE FROM sync_state
    WHERE entity_kind IN ('agent', 'workflow', 'trigger', 'settings', 'connectors');
DELETE FROM capability_index;

-- chief-of-staff is now nandi. Agent IDs, on this side and on teammates'
-- (their Nandi upgrades the same way).
UPDATE conversations SET agent_id = 'nandi' WHERE agent_id = 'chief-of-staff';
UPDATE runs SET agent_id = 'nandi' WHERE agent_id = 'chief-of-staff';
UPDATE agent_queue SET agent_id = 'nandi' WHERE agent_id = 'chief-of-staff';
UPDATE approvals SET agent_id = 'nandi' WHERE agent_id = 'chief-of-staff';
UPDATE harness_sessions SET agent_id = 'nandi' WHERE agent_id = 'chief-of-staff';
UPDATE delegations SET target_agent = 'nandi' WHERE target_agent = 'chief-of-staff';
UPDATE delegations SET requester_agent = 'nandi' WHERE requester_agent = 'chief-of-staff';
UPDATE tasks SET requester_agent = 'nandi' WHERE requester_agent = 'chief-of-staff';
UPDATE tasks SET provider_agent = 'nandi' WHERE provider_agent = 'chief-of-staff';
UPDATE request_returns SET provider_agent = 'nandi' WHERE provider_agent = 'chief-of-staff';
UPDATE eta_history SET provider_agent = 'nandi' WHERE provider_agent = 'chief-of-staff';
UPDATE capability_evidence SET provider_id = 'nandi' WHERE provider_id = 'chief-of-staff';
UPDATE capabilities_projection SET owner_id = 'nandi' WHERE owner_id = 'chief-of-staff';
UPDATE request_steps SET target = 'nandi'
    WHERE kind = 'localAgent' AND target = 'chief-of-staff';

-- Tables keyed by agent
UPDATE OR IGNORE agent_state SET agent_id = 'nandi' WHERE agent_id = 'chief-of-staff';
UPDATE OR IGNORE agent_memory SET agent_id = 'nandi' WHERE agent_id = 'chief-of-staff';
UPDATE OR IGNORE trust_policies SET provider_agent = 'nandi'
    WHERE provider_agent = 'chief-of-staff';
UPDATE OR IGNORE blocked_members SET agent_id = 'nandi' WHERE agent_id = 'chief-of-staff';
UPDATE OR IGNORE capability_providers SET provider_id = 'nandi'
    WHERE provider_id = 'chief-of-staff';
UPDATE OR IGNORE capability_aggregates SET provider_id = 'nandi'
    WHERE provider_id = 'chief-of-staff';
UPDATE OR IGNORE provider_circuits SET provider_id = 'nandi'
    WHERE provider_id = 'chief-of-staff';

-- RequestSource `chiefOfStaff` is now `nandi`
UPDATE requests SET source = 'nandi' WHERE source = 'chiefOfStaff';
