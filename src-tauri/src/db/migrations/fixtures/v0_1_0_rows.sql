-- Representative rows of a person's state.db written by Nandi 0.1.0
-- (schema version 2, migration 002). The migration tests load these into a
-- version 2 database and check each row after the upgrade: kept as it was,
-- renamed from chief-of-staff to nandi, or gone with its removed feature.
-- Keep it at 0.1.0: rows for a later table go in a fixture of their own.

-- Chat: the Chief of Staff's conversation and one with the person's agent
INSERT INTO conversations (id, agent_id, created_at, title) VALUES
    ('conv-cos', 'chief-of-staff', 1000, NULL),
    ('conv-writer', 'writer', 1050, 'Blog post');

INSERT INTO messages (id, conversation_id, run_id, role, content, created_at, metadata) VALUES
    ('m-user', 'conv-cos', 'run-1', 'user', 'Summarize my mail', 1001, '{"attachments":[]}'),
    ('m-reply', 'conv-cos', 'run-1', 'assistant', 'Three new mails.', 1002, '{}'),
    ('m-approval', 'conv-cos', 'run-1', 'approval', '', 1003,
        '{"tool":"shell","args":{"cmd":"ls"},"status":"approved","expiresAt":5000}'),
    ('m-writer', 'conv-writer', 'run-3', 'user', 'Draft the intro', 1051, '{}');

-- Runs, their events, an approval and a queued run
INSERT INTO runs (id, agent_id, conversation_id, started_at, ended_at, status, input, summary,
                  input_tokens, output_tokens, cost_usd, model, source_type, source_id,
                  source_label, priority, queued_at, queue_position, parent_run_id,
                  compaction_summary, runtime) VALUES
    ('run-1', 'chief-of-staff', 'conv-cos', 1001, 1010, 'completed', 'Summarize my mail',
     'Done', 120, 30, 0.25, 'deepseek:deepseek-flash', 'user', NULL, NULL, 100, NULL, NULL,
     NULL, NULL, 'native'),
    ('run-2', 'chief-of-staff', 'conv-cos', 2000, NULL, 'queued', 'Morning digest', NULL,
     0, 0, NULL, NULL, 'scheduled', 'morning-digest', 'Morning digest', 50, 2000, 0,
     'run-1', 'Earlier: three mails', NULL),
    ('run-3', 'writer', 'conv-writer', 1051, 1060, 'completed', 'Draft the intro', 'Drafted',
     80, 200, 0.1, NULL, 'user', NULL, NULL, 100, NULL, NULL, NULL, NULL, 'native');

INSERT INTO run_events (id, run_id, kind, payload, created_at) VALUES
    (1, 'run-1', 'started', '{}', 1001),
    (2, 'run-1', 'completed', '{"summary":"Done"}', 1010);

INSERT INTO approvals (id, agent_id, run_id, message_id, tool_call_id, tool, args, status,
                       created_at, resolved_at, expires_at, decided_by) VALUES
    ('m-approval', 'chief-of-staff', 'run-1', 'm-approval', 'call-1', 'shell', '{"cmd":"ls"}',
     'approved', 1003, 1004, 5000, 'Ada');

INSERT INTO agent_queue (id, agent_id, run_id, source_type, source_id, source_label, priority,
                         message, message_id, attachments, created_at) VALUES
    ('q-1', 'chief-of-staff', 'run-2', 'scheduled', 'morning-digest', 'Morning digest', 50,
     'Morning digest', 'm-queued', NULL, 2000);

-- Agent health and the memory cache
INSERT INTO agent_state (agent_id, status, error, updated_at) VALUES
    ('chief-of-staff', 'idle', NULL, 1010),
    ('writer', 'error', 'container stopped', 1060);

INSERT INTO agent_memory (agent_id, idx, content, created_at, source_run) VALUES
    ('chief-of-staff', 0, 'Ada prefers short replies', 1010, 'run-1');

-- Triggers: last fire and a fire record
INSERT INTO trigger_state (trigger_id, last_fired_at) VALUES ('morning-digest', 2000);

INSERT INTO trigger_fires (id, trigger_id, fired_at, payload, run_id, status, error) VALUES
    ('fire-1', 'morning-digest', 2000, '{"at":"08:00"}', 'run-2', 'queued', NULL);

-- Machine-local settings, as 0.1.0 seeded and wrote them
INSERT INTO local_settings (key, value) VALUES
    ('onboarding.step', '"done"');

-- The relay: this install's ID, its instance token and the members seen,
-- each value JSON
INSERT INTO relay_state (key, value) VALUES
    ('instance_id', '"inst-me"'),
    ('relay_token', '"tok-0.1.0"'),
    ('relay_token_relay', '"wss://relay.example.test/relay"'),
    ('network_members', '[]');

-- The projection of the gold copy, rebuilt from the files after the upgrade
INSERT INTO agents (id, name, description, model, image, capabilities, definition,
                    created_at, updated_at, source_path, projection_version) VALUES
    ('chief-of-staff', 'Chief of Staff', 'Routes the ask box', NULL, NULL, '{}', '{}',
     900, 900, '.nandi/agents/chief-of-staff/skill.md', 1);

INSERT INTO settings (key, value, source_path, projection_version) VALUES
    ('connector_grants', '[{"agent":"chief-of-staff","connector":"outlook","access":"read"}]',
     '.nandi/settings.yaml', 1);

INSERT INTO sync_state (entity_kind, entity_id, source_path, file_hash, last_projected_at)
VALUES
    ('agent', 'chief-of-staff', '.nandi/agents/chief-of-staff/skill.md', 'abc', 900),
    ('settings', 'settings', '.nandi/settings.yaml', 'def', 900);

INSERT INTO capability_index (entity_id, capability, agent_card, updated_at) VALUES
    ('chief-of-staff', 'triage', '{"id":"chief-of-staff"}', 900);

-- The Inbox: a gateway hold and a manual workflow step
INSERT INTO gateway_holds (task_id, caller_instance, caller_name, capability, summary, status,
                           created_at, expires_at, decided_at) VALUES
    ('task-in', 'inst-sam', 'Sam Lee', 'ask-me', 'Can you send the Q3 numbers?', 'pending',
     1100, 90000, NULL);

INSERT INTO workflow_runs (id, workflow_id, status, started_at, ended_at, context) VALUES
    ('wfr-1', 'pay-invoice', 'waiting', 1300, NULL, '{"invoice":42}');

INSERT INTO manual_steps (run_id, step_id, workflow_id, status, instructions, deadline,
                          created_at, completed_at) VALUES
    ('wfr-1', 'sign', 'pay-invoice', 'pending', 'Sign the payment', 99000, 1300, NULL);

-- A Request from the ask box, with a step the Chief of Staff does
INSERT INTO requests (id, title, source, ask_text, plan, status, error, created_at,
                      updated_at, ended_at) VALUES
    ('req-1', 'Pay invoice 42', 'me', 'Please pay invoice 42', '{"steps":[]}', 'running',
     NULL, 1250, 1300, NULL);

INSERT INTO request_steps (request_id, position, kind, target, label, depends_on, status,
                           run_id, workflow_run_id, task_id, error, started_at, ended_at) VALUES
    ('req-1', 0, 'localAgent', 'chief-of-staff', 'Find the invoice', '[]', 'done', 'run-1',
     NULL, NULL, NULL, 1250, 1260),
    ('req-1', 1, 'workflow', 'pay-invoice', 'Pay it', '[0]', 'running', NULL, 'wfr-1', NULL,
     NULL, 1300, NULL);

-- Person-to-person work: a task, its transitions, a delegation, a trust
-- policy and a reassignment
INSERT INTO tasks (id, requester_instance, requester_agent, provider_instance, provider_agent,
                   capability, input, state, eta_seconds, deadline, priority, created_at,
                   updated_at, acknowledged_at, completed_at, error, result, parent_task_id,
                   source_kind, attempt_number, previous_attempt_id) VALUES
    ('task-out', 'inst-me', 'chief-of-staff', 'inst-sam', 'finance', 'q3-numbers',
     '{"quarter":3}', 'completed', 600, 99000, 1, 1500, 1600, 1510, 1600, NULL,
     '{"total":12}', NULL, 'direct', 1, NULL);

INSERT INTO task_events (id, task_id, from_state, to_state, reason, reason_kind, stage, at) VALUES
    (1, 'task-out', NULL, 'submitted', NULL, NULL, NULL, 1500),
    (2, 'task-out', 'submitted', 'completed', 'Sent', 'done', 'delivery', 1600);

INSERT INTO delegations (id, direction, capability, target_agent, requester_agent,
                         provider_owner, requester_owner, input, output, status, started_at,
                         ended_at, error, tokens_in, tokens_out, parent_run_id) VALUES
    ('del-1', 'outbound', 'q3-numbers', 'finance', 'chief-of-staff', 'Sam Lee', 'Ada',
     '{"quarter":3}', '{"total":12}', 'completed', 1500, 1600, NULL, 40, 10, 'run-1');

INSERT INTO trust_policies (requester, provider_agent, policy, updated_at) VALUES
    ('inst-sam', 'chief-of-staff', 'allow', 1400);

INSERT INTO reassigned_tasks (task_id, from_instance, to_instance, reason, new_task_id,
                              reassigned_at) VALUES
    ('task-old', 'inst-sam', 'inst-kim', 'On leave', 'task-new', 1700);

INSERT INTO capability_evidence (id, capability_id, capability_version, task_id,
                                 provider_instance, provider_kind, provider_id, role, status,
                                 duration_ms, tokens_in, tokens_out, cost_usd,
                                 clarification_rounds, escalation_required, approval_required,
                                 user_rating, step_breakdown, observed_at, metadata) VALUES
    ('ev-1', 'q3-numbers', '1', 'task-out', 'inst-sam', 'agent', 'finance', 'requester',
     'completed', 100000, 40, 10, 0.01, 0, 0, 1, 5, NULL, 1600, NULL);

-- Removed features: their rows go with their tables
INSERT INTO teams (id, name, github_org, registry_repo, created_at, source_kind,
                   source_identifier) VALUES
    ('team-1', 'Finance', 'acme', 'acme/registry', 800, 'github', NULL);

INSERT INTO team_members (team_id, github_handle, display_name, role, joined_at) VALUES
    ('team-1', 'sam', 'Sam Lee', 'member', 800);

INSERT INTO circuit_breakers (instance_id, capability, consecutive_failures, opened_at) VALUES
    ('inst-sam', 'q3-numbers', 1, NULL);
