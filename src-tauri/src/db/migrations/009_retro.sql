-- Migration 009: the weekly retro's proposals (`services::retro`).
--
-- Once a week Nandi reads the person's corrections and proposes at most
-- three changes to an agent's definitions. Each proposal waits in the
-- Inbox; a yes writes one gold copy commit, Undo writes another.
--
-- `target`: `skill`, `playbook:<name>`, `workflow:<id>` or
-- `trigger:<id>`; `path` its file in the gold copy. `before` is the file
-- when proposed (NULL: a new playbook), `after` the proposed file.
-- `runs`: a JSON array of the run IDs it learned from.
-- `status`: `pending`, `applied`, `refused`, `undone` or `kept`.
-- `commit_id` / `undo_commit_id`: the gold copy commits. `verdict` (set
-- once by the check): `helped`, `same` or `worse`, with the rates it
-- compared in `rates` (JSON).

CREATE TABLE retro_proposals (
    id             TEXT PRIMARY KEY,
    run_id         TEXT NOT NULL,
    agent_id       TEXT NOT NULL,
    target         TEXT NOT NULL,
    path           TEXT NOT NULL,
    reason         TEXT NOT NULL,
    runs           TEXT NOT NULL DEFAULT '[]',
    before         TEXT,
    after          TEXT NOT NULL,
    diff           TEXT NOT NULL,
    status         TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    decided_at     INTEGER,
    commit_id      TEXT,
    undo_commit_id TEXT,
    verdict        TEXT,
    rates          TEXT
);
CREATE INDEX retro_proposals_status ON retro_proposals(status, created_at);
