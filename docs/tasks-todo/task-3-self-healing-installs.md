# Task: Existing installations heal themselves

**From:** owner, 2026-11-07, high priority. Users must never need us, their admins or the world to get Nandi working again after an update or a local problem.
**Trigger:** a dev home that ran the first intake build failed to start after the next one: `Failed to initialize database: no such column: thread_id` (a table created by an older build, then an index on a new column). In a released app that is a crash on launch with no way out.
**Size:** L. Startup and data code, so `pnpm run test:offline` must stay green.

## The problem

- Nandi 0.1.0 is released (2026-11-04), so real installations have data from older builds.
- The schema is `CREATE TABLE IF NOT EXISTS`: a table from an older build keeps its old columns, and anything that needs a new column fails.
- A setup error panics (`tauri::app.rs`: "Failed to setup app"). The user sees nothing, or a crash dialog.

## Decision needed first

- [ ] **"No legacy code" after the release.** The workspace rule says no migrations while nothing is released. 0.1.0 is out, so schema changes now need a way forward for existing data. Proposal: versioned, forward-only schema steps for **run data** only (runs, messages, requests, intake queue); everything that is a projection of the gold copy is rebuilt, never migrated. Still no old-name aliases, serde aliases or shims.

## 1. The database repairs itself at startup

- [ ] **Schema version** in `PRAGMA user_version`; each release's steps run once, in order, in a transaction, after a backup (`state.db.bak-<version>`).
- [ ] **Drift check** after the steps: compare each table's columns (`PRAGMA table_info`) with the schema. A missing column is added when it has a default; otherwise the table is rebuilt (copy, recreate, copy back) for run data, or dropped and re-projected for projections.
- [ ] **Projections rebuild** from the gold copy (`~/.nandi/repo/.nandi/`) when they can't be repaired: agents, workflows, triggers, connectors, settings. Run history is kept.
- [ ] **Corrupt or locked file:** move it aside, start fresh, rebuild projections, tell the user what was kept and what wasn't.
- [ ] Test fixtures: a `state.db` from each released version, opened by the current build. A release can't ship until its predecessor's fixture opens.

## 2. Startup never crashes

- [ ] A setup error opens a **recovery screen**, not a panic: what went wrong in plain words, **Repair** (section 1, then restart), **Start fresh** (keeps the gold copy and notes), and **Export diagnostics**.
- [ ] Every startup step that can fail reports through it (database, repo, keyring, relay never: it doesn't block startup).
- [ ] The companion shows the same state if the main window can't open.

## 3. A model helps when the rules can't

- [ ] When automatic repair fails, Nandi asks a model to diagnose it: the user's default model, else the free trial through Nandi's gateway (`services/llm_trial/`), else no model: the recovery screen and the built-in help guide (`services/help/`).
- [ ] Input: the error, the schema drift report and recent log lines, **redacted**: no secrets, tokens, notes, email or message content, file contents.
- [ ] Output: a choice from a fixed list of safe actions (back up, add a column, rebuild a table, rebuild projections, clear a cache, re-run a sign-in, start fresh), each shown to the user with what it changes, and run only after they approve. Never free-form SQL or shell commands.
- [ ] The same path for user-facing errors elsewhere (a connector that won't start, a sign-in that expired, an image that won't pull): plain words, a suggested fix, one click.

## 4. What the user sees

- [ ] No stack traces, no "contact your admin". One line: what happened, what Nandi did, what (if anything) the user needs to do.
- [ ] After a silent repair: a toast, "Nandi fixed a problem with its local data", with details on click.
- [ ] **Export diagnostics**: a zip of redacted logs, the drift report and versions, for the rare case a person must look.

## Tests

- Old-schema fixture opens and gains the new columns; a table that can't take a column is rebuilt with its rows.
- Corrupt `state.db`: moved aside, app starts, projections rebuilt.
- Setup error: recovery screen, no panic (unit test on the setup result, manual for the window).
- Model-assisted repair: the prompt is redacted; only listed actions can run; nothing runs without approval.
- `pnpm run test:offline` still boots with no relay.

## Dev note

The dev app's home comes from `NANDI_HOME` or its default; the log names it (`services::watcher` "Watching <home>\repo"). The 2026-11-07 failure was in `D:\tmp\nandi-test-1107\.home-dev`; the stale `intake_queue` table there was dropped by hand.
