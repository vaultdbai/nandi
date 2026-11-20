-- Migration 008: the user's to-dos are assignments too
-- (`services::gateway::todos`). Nandi, or the user, opens one for the user
-- from mail, an event or a chat: a local assignment, `source` set, no
-- remote caller. Work other people assign keeps `source` NULL.
--
-- `kind`: `do` (an action), `check` (verify and close) or `reminder` (a
-- date to remember). Every existing assignment is `do`.
-- `source`: `email`, `event`, `chat` or `manual`; `source_ref` names where
-- it was read (an email's message ID), so the same place read twice is
-- one to-do. `related_ref`: an Inbox item Nandi prepared for it (an
-- approval, a Request). `run_id` / `assigned_agent`: the run of the agent
-- the user handed it to.

ALTER TABLE gateway_assignments ADD COLUMN kind TEXT NOT NULL DEFAULT 'do';
ALTER TABLE gateway_assignments ADD COLUMN source TEXT;
ALTER TABLE gateway_assignments ADD COLUMN source_ref TEXT;
ALTER TABLE gateway_assignments ADD COLUMN related_ref TEXT;
ALTER TABLE gateway_assignments ADD COLUMN run_id TEXT;
ALTER TABLE gateway_assignments ADD COLUMN assigned_agent TEXT;
CREATE UNIQUE INDEX gateway_assignments_source_ref
    ON gateway_assignments(source, source_ref) WHERE source_ref IS NOT NULL;
