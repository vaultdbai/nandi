-- Representative rows of a person's state.db at the baseline (migration
-- 001). The migration tests load these into a baseline database and check
-- every row survives the whole chain. Keep it at the baseline: rows for a
-- later table go in a fixture of their own.

-- Chat: an agent conversation, an Inbox Ask thread and an intake thread
INSERT INTO conversations (id, agent_id, created_at, title) VALUES
    ('conv-agent', 'nandi', 1000, NULL),
    ('inbox:gatewayHold:task-in', 'nandi', 1100, 'Q3 numbers'),
    ('intake:in-1', 'nandi', 1200, 'Invoice from Acme');

INSERT INTO messages (id, conversation_id, run_id, role, content, created_at, metadata) VALUES
    ('m-user', 'conv-agent', 'run-1', 'user', 'Summarize my mail', 1001, '{"attachments":[]}'),
    ('m-reply', 'conv-agent', 'run-1', 'assistant', 'Three new mails.', 1002, '{}'),
    ('m-approval', 'conv-agent', 'run-1', 'approval', '', 1003,
        '{"tool":"shell","args":{"cmd":"ls"},"status":"approved","expiresAt":5000}'),
    ('m-thread', 'inbox:gatewayHold:task-in', NULL, 'user', 'Why now?', 1101, '{}');

-- Runs, their events and an approval
INSERT INTO runs (id, agent_id, conversation_id, started_at, ended_at, status, input, summary,
                  input_tokens, output_tokens, cost_usd, model, source_type, source_id,
                  source_label, priority, queued_at, queue_position, parent_run_id,
                  compaction_summary, runtime) VALUES
    ('run-1', 'nandi', 'conv-agent', 1001, 1010, 'completed', 'Summarize my mail', 'Done',
     120, 30, 0.25, 'deepseek:deepseek-flash', 'user', NULL, NULL, 100, NULL, NULL, NULL,
     NULL, 'native'),
    ('run-2', 'nandi', 'conv-agent', 2000, NULL, 'queued', 'Morning digest', NULL,
     0, 0, NULL, NULL, 'scheduled', 'morning-digest', 'Morning digest', 50, 2000, 0,
     'run-1', 'Earlier: three mails', NULL);

INSERT INTO run_events (id, run_id, kind, payload, created_at) VALUES
    (1, 'run-1', 'started', '{}', 1001),
    (2, 'run-1', 'completed', '{"summary":"Done"}', 1010);

INSERT INTO approvals (id, agent_id, run_id, message_id, tool_call_id, tool, args, status,
                       created_at, resolved_at, expires_at, decided_by) VALUES
    ('m-approval', 'nandi', 'run-1', 'm-approval', 'call-1', 'shell', '{"cmd":"ls"}',
     'approved', 1003, 1004, 5000, 'Ada');

-- Triggers: last fire and a fire record
INSERT INTO trigger_state (trigger_id, last_fired_at) VALUES ('morning-digest', 2000);

INSERT INTO trigger_fires (id, trigger_id, fired_at, payload, run_id, status, error) VALUES
    ('fire-1', 'morning-digest', 2000, '{"at":"08:00"}', 'run-2', 'queued', NULL),
    ('fire-2', 'mail-check', 2100, '{}', NULL, 'error', 'mailbox unreachable');

-- Machine-local settings, with an email cursor
INSERT INTO local_settings (key, value) VALUES
    ('email.gmail.cursor', '{"historyId":"98765"}'),
    ('onboarding.step', '"done"');

-- The Inbox: an intake item, a gateway hold and a manual workflow step
INSERT INTO intake_queue (id, at, source, action, request_id, request_title, text, sender,
                          screening, status, result_request_id, thread_id, plan) VALUES
    ('in-1', 1200, 'email', 'start', NULL, NULL, 'Please pay invoice 42', 'billing@acme.test',
     '{"verdict":"work"}', 'pending', NULL, 'intake:in-1', '{"steps":[]}');

INSERT INTO gateway_holds (task_id, caller_instance, caller_name, capability, summary, status,
                           created_at, expires_at, decided_at) VALUES
    ('task-in', 'inst-sam', 'Sam Lee', 'ask-me', 'Can you send the Q3 numbers?', 'pending',
     1100, 90000, NULL);

INSERT INTO workflow_runs (id, workflow_id, status, started_at, ended_at, context) VALUES
    ('wfr-1', 'pay-invoice', 'waiting', 1300, NULL, '{"invoice":42}');

INSERT INTO manual_steps (run_id, step_id, workflow_id, status, instructions, deadline,
                          created_at, completed_at) VALUES
    ('wfr-1', 'sign', 'pay-invoice', 'pending', 'Sign the payment', 99000, 1300, NULL);

INSERT INTO requests (id, title, source, ask_text, plan, status, error, created_at,
                      updated_at, ended_at) VALUES
    ('req-1', 'Pay invoice 42', 'email', 'Please pay invoice 42', '{"steps":[]}', 'waiting',
     NULL, 1250, 1300, NULL);

-- Person-to-person work: a task, its transitions and a delegation
INSERT INTO tasks (id, requester_instance, requester_agent, provider_instance, provider_agent,
                   capability, input, state, eta_seconds, deadline, priority, created_at,
                   updated_at, acknowledged_at, completed_at, error, result, parent_task_id,
                   source_kind, attempt_number, previous_attempt_id) VALUES
    ('task-out', 'inst-me', 'nandi', 'inst-sam', 'finance', 'q3-numbers', '{"quarter":3}',
     'completed', 600, 99000, 1, 1500, 1600, 1510, 1600, NULL, '{"total":12}', NULL,
     'direct', 1, NULL);

INSERT INTO task_events (id, task_id, from_state, to_state, reason, reason_kind, stage, at) VALUES
    (1, 'task-out', NULL, 'submitted', NULL, NULL, NULL, 1500),
    (2, 'task-out', 'submitted', 'completed', 'Sent', 'done', 'delivery', 1600);

INSERT INTO delegations (id, direction, capability, target_agent, requester_agent,
                         provider_owner, requester_owner, input, output, status, started_at,
                         ended_at, error, tokens_in, tokens_out, parent_run_id) VALUES
    ('del-1', 'outbound', 'q3-numbers', 'finance', 'nandi', 'Sam Lee', 'Ada', '{"quarter":3}',
     '{"total":12}', 'completed', 1500, 1600, NULL, 40, 10, 'run-1');
