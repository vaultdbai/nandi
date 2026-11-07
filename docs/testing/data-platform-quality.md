# Data, gold copy, storage, platform and quality gates

This section covers where Nandi keeps its data (the gold-copy git repo at `~/.nandi/repo/.nandi/`, the SQLite projection at `~/.nandi/local/state.db`, the OS keyring, and `~/.nandi/config.json`), how the projection stays in step with the files, the optional git backup remote, the home folder and `NANDI_*` environment variables, the enterprise LLM config, the app binary's CLI and headless mode, platform specifics, build/release/updater, and the developer quality gates.

Conventions used below: `HOME` means `~/.nandi` (or `NANDI_HOME`). "Repo" means `HOME/repo`. For a disposable test profile, set `NANDI_HOME` to a temp folder. The OS keyring is NOT isolated by `NANDI_HOME`: it is keyed by the bundle identifier `com.nandi.app` (see DT-35).

## Gold copy: layout and write path

### DT-01 — Gold-copy repo layout and first-run initialization

- **What:** Definitions live as files in a git repo at `HOME/repo`: `.nandi/manifest.yaml`, `.nandi/settings.yaml`, `.nandi/connectors.yaml`, `.nandi/agents/<id>/{skill.md,instructions.md,workflows/*.yaml}`, `.nandi/workflows/*.yaml`, `.nandi/triggers/*.yaml`, `.nandi/capabilities/*.yaml`, `.nandi/policies/*.md`, `.nandi/installed.yaml`, and vendored marketplace folders under `.nandi/agents/marketplace/`. A brand-new install gets `git init` (branch `main`), a `.gitignore`, a manifest, and one commit ("Initialize Nandi repo"). On every start, whatever the repo lacks of `.nandi/settings.yaml`, `.gitignore` and `.nandi/manifest.yaml` is written (the manifest last).
- **Where:** `src-tauri/src/services/repos/layout.rs`, `services/repos/mod.rs`, `services/repo_bootstrap.rs` (`install_layout`, `bootstrap`, `adopt_repo`), `services/domain/repo.rs` (`REPO_SCHEMA_VERSION = 1`); docs `docs/developer/data-model.md`, `domain-model.md`.
- **Preconditions:** None (fresh `NANDI_HOME`).
- **Steps:**
  1. Set `NANDI_HOME` to an empty temp folder and start the app (or `nandi --headless`).
  2. Run `git -C $NANDI_HOME/repo log --oneline` and `git status`.
  3. List `$NANDI_HOME/repo/.nandi/`.
- **Expected:** `repo/` is a git repo on `main` with one initial commit; `.nandi/manifest.yaml` has `schema_version: 1` and an owner; `.gitignore` excludes `.nandi/local/`, `*.db*`, `*.log`, `work/`, and atomic-write temp files; `HOME/local/state.db`, `HOME/local/logs/`, `HOME/local/work/` exist.
- **Edge / negative cases:**
  - Restart: no second init commit, nothing changes (idempotent).
  - Delete `.gitignore` and restart: it is written again and staged, not committed.
  - An unrelated git repo around `HOME` is never opened (only the repo at the layout root is used).
- **Automation:** Covered by: `services/repo_bootstrap.rs` `a_new_install_gets_a_fresh_committed_repo`, `settings_refer_to_keyring_entries_only`; `services/repos/layout.rs` `paths_live_under_dot_nandi` | Layer: unit | Gap: no API-level test that boots a fresh home and asserts the tree.
- **Needs relay:** no
- **Status:** Shipped

### DT-02 — Atomic, staged writes for every definition change

- **What:** Every app write to the repo is atomic (temp file `.nandi-tmp-*` in the same folder, then rename) and skipped when content is identical; the change is then `git add`-ed so `git status` shows exactly what the app changed. Nothing is auto-committed by ordinary edits. Write path: domain validates, repository writes the file, `RepoStore::commit` updates aggregate + projection in one transaction and publishes `repo://<kind>-<action>`.
- **Where:** `services/repos/fs_atomic.rs` (`write_if_changed`, `TEMP_MARKER`), `services/repos/git.rs` (`GitIndex`), `services/repo_store.rs` (`commit`), `services/repos/vendor_repo.rs` (whole-folder swap via hidden staging folder for marketplace installs).
- **Preconditions:** Repo initialized.
- **Steps:**
  1. Change something that is a definition (e.g. Settings › Appearance theme, or edit and save an agent).
  2. Run `git -C HOME/repo status --short`.
  3. Save the same value again.
- **Expected:** The changed file shows as staged (`M `/`A `), no temp files remain, no commit was made; saving identical content changes nothing (no new event, no new staged change).
- **Edge / negative cases:**
  - Kill the app mid-save: target file is either old or new, never half-written.
  - Git staging failure is logged, file on disk still updated.
- **Automation:** Covered by: `services/repos/fs_atomic.rs` `writes_atomically_and_skips_identical_content`, `hashes_are_stable_hex`; `services/repos/git.rs` test; `services/repo_service.rs` `saving_writes_the_files_projects_and_publishes` | Layer: unit | Gap: none significant.
- **Needs relay:** no
- **Status:** Shipped

### DT-03 — Portable settings in `.nandi/settings.yaml`

- **What:** Theme, palette, language, default model, notification prefs, agent defaults, marketplace settings, per-agent consent, delegation preferences and `secret_refs` are stored in the repo, so they follow the user to other machines. Unknown top-level keys and secret-looking values are rejected, so a pasted API key fails validation instead of being committed.
- **Where:** `services/domain/settings.rs` (`Settings`, `SettingsFile` with `deny_unknown_fields`), `services/repos/settings_repo.rs`, `services/preferences.rs` (overlay + write-back, emits `repo://settings-updated`).
- **Preconditions:** None.
- **Steps:**
  1. Settings › Appearance: switch theme to Dark and language to another locale.
  2. Open `HOME/repo/.nandi/settings.yaml`.
  3. Hand-edit `theme: light` in the file and save.
  4. Add `api_key: sk-123` at top level and save.
- **Expected:** Step 2 shows `theme: dark` and the language; step 3 flips the UI theme live (via watcher); step 4 is rejected (`repo://validation_failed`, last good settings stay in effect, a problem appears in Settings › Backup › This computer’s copy).
- **Edge / negative cases:**
  - Empty file = defaults (default model `claude:claude-sonnet-5`).
  - Invalid `delegation.preferred_providers` card ID rejected.
  - `secret_refs` are preserved verbatim, never resolved into the file.
- **Automation:** Covered by: `services/domain/settings.rs` (`empty_file_is_defaults`, `rejects_secrets_and_bad_values`, `preserves_secret_refs_without_resolving_them`, `provider_pins_round_trip_and_are_validated`, `consent_round_trips_and_needs_agent_ids`, `registries_default_validate_and_round_trip`, `mutations_keep_invariants`), `services/repos/settings_repo.rs` test | Layer: unit; api for the write-back via `PUT /api/preferences` | Gap: no test of the hand-edit-to-UI round trip.
- **Needs relay:** no
- **Status:** Shipped

### DT-04 — Secrets only in the OS keyring

- **What:** LLM keys, AWS keys, trigger secrets, the GitHub token and the backup credential go to the OS keychain (macOS Keychain, Windows Credential Manager, Linux Secret Service) under service `com.nandi.app`; repo files hold references only (`nandi.llm.<provider>`, `nandi.trigger.<id>`, `nandi.github.token`, `nandi.remote.credential`). An unreadable entry reads as missing (logged, never echoed).
- **Where:** `services/secrets.rs` (`Secrets::keychain`, `get`, `read`, `set`, `delete`), `services/llm_keys.rs`, `services/remote/credentials.rs`.
- **Preconditions:** Desktop session with a keyring (Linux: Secret Service running).
- **Steps:**
  1. Save a provider key in Settings › Models & runtimes.
  2. `git -C HOME/repo grep -n "<part of the key>"` and inspect `settings.yaml`.
  3. Inspect the OS credential store for `com.nandi.app` / `nandi.llm.<provider>` (Windows: `cmdkey /list:nandi.llm.<provider>.com.nandi.app`).
- **Expected:** The key exists only in the keyring; the repo has at most a `secret_refs` entry; no key in `config.json` or `state.db`.
- **Edge / negative cases:**
  - Linux without Secret Service: saving a key fails with an error, nothing written to disk.
  - A corrupt keyring entry: treated as missing by `get`, error from `read`.
- **Automation:** Covered by: `services/secrets.rs` `in_memory_round_trip`, `an_unreadable_entry_is_missing_to_get_and_an_error_to_read` | Layer: unit (in-memory), manual (real keychain per OS) | Gap: no test scanning the repo for leaked secrets after a full save flow.
- **Needs relay:** no
- **Status:** Shipped

### DT-05 — Repo format version guard

- **What:** `.nandi/manifest.yaml` carries `schema_version`. A repo written by a newer Nandi (or with `schema_version: 0`) is refused with "...this version of Nandi reads up to 1. Update Nandi."
- **Where:** `services/domain/repo.rs` (`REPO_SCHEMA_VERSION`, manifest check).
- **Preconditions:** Repo initialized.
- **Steps:**
  1. Quit the app; set `schema_version: 2` in `.nandi/manifest.yaml`.
  2. Start the app and check the log / Settings › Backup › This computer’s copy problems.
- **Expected:** Manifest reported as an error containing "Update Nandi"; the app does not rewrite the manifest.
- **Edge / negative cases:** `schema_version: 0` also refused; restoring `1` and editing again recovers live (watcher).
- **Automation:** Covered by: `services/domain/repo.rs` `manifests_from_newer_versions_are_rejected` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-06 — Load an entity as stored in its file

- **What:** Returns one entity as JSON exactly as its file holds it, for editors (`kind`: `agent` | `workflow` | `trigger` | `connector` | `settings`).
- **Where:** Tauri `repo_load_entity` (`src-tauri/src/commands/repo.rs`), `GET /api/repo/entities/{kind}/{id}` (`src-tauri/src/http/repo.rs`); used by `src/hooks/useAgentEditor.ts`.
- **Preconditions:** At least one agent.
- **Steps:**
  1. `GET /api/repo/entities/agent/<id>` with the web token.
  2. Repeat for a workflow, trigger, `connector`, and `settings/settings`.
- **Expected:** 200 with the file's fields; unknown kind or id returns an error string.
- **Edge / negative cases:** Bad kind (`foo`); id with path traversal (`../x`) rejected by ID validation.
- **Automation:** Covered by: `services/repo_service.rs` `loads_other_entities_as_json`; `src-tauri/src/http/repo.rs` test | Layer: api | Gap: no negative-kind test.
- **Needs relay:** no
- **Status:** Shipped

### DT-07 — Save an agent definition through the repo (cross-ref Agents)

- **What:** Writes an agent's `skill.md`/`instructions.md` from the editor input, creating the folder if new; validation happens before any write; marketplace agents are read-only until forked; fields the editor doesn't edit are kept. Listed here for the write path only; the editor UI is in the Agents section.
- **Where:** Tauri `repo_save_agent`; `POST /api/repo/agents/{id}/save`; `services/repo_service.rs` (`save_agent`, `save_agent_definition`).
- **Preconditions:** None.
- **Steps:** Save an agent via the editor or the route; check `git status` and `GET /api/agents`.
- **Expected:** File staged, projected row updated, `repo://agent-created|updated` emitted.
- **Edge / negative cases:** Invalid model / ID → error, nothing written; saving a marketplace agent → refused.
- **Automation:** Covered by: `services/repo_service.rs` `saving_writes_the_files_projects_and_publishes`, `saving_an_unknown_agent_creates_it_and_invalid_saves_change_nothing`, `invalid_definitions_are_rejected_before_any_write`, `definitions_keep_what_the_editor_does_not_edit`, `deleting_removes_the_folder_and_the_projected_row` | Layer: unit, api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## SQLite projection and rebuild

### DT-08 — Startup load with drift detection

- **What:** At startup `RepoStore::load` reads every file. If the projection is empty or built by a different `PROJECTION_VERSION` (currently 2), it rebuilds fully; otherwise it re-projects only entities whose file hash differs from `sync_state` (edited while the app was closed) and drops rows whose file is gone. Runtime tables (runs, approvals, fires, `trigger_state`) are never touched.
- **Where:** `services/repo_store.rs` (`load`), `services/projection.rs` (`rebuild_all`, `project_entity`, `PROJECTION_VERSION`), `services/mod.rs` (`AppState::start`).
- **Preconditions:** An install with agents and some run history.
- **Steps:**
  1. Quit the app. Edit an agent's `skill.md` description; delete a trigger file.
  2. Start the app.
- **Expected:** The agent shows the new description, the trigger is gone, run history intact; log line "Loaded the repo: N agents, M workflows re-projected".
- **Edge / negative cases:**
  - Delete `HOME/local/state.db` entirely: everything definitional comes back (run history does not).
  - A file that became invalid while closed: reported as a problem, last projected version kept.
- **Automation:** Covered by: `services/repo_store.rs` `startup_reprojects_only_drifted_entities`, `loads_reloads_and_keeps_the_last_good_version`; `services/projection.rs` `a_stale_projection_version_forces_a_rebuild`, `rebuilds_everything_in_one_transaction`, `projects_and_removes_single_entities`; `services/repo_service.rs` `a_fresh_database_is_rebuilt_from_files_on_load` | Layer: unit | Gap: no automated restart test on a real home.
- **Needs relay:** no
- **Status:** Shipped

### DT-09 — Rebuild the projection (command and HTTP)

- **What:** Wipes projected tables (`agents`, `workflows`, `triggers`, `settings`, `connectors`, `sync_state`) and re-reads every file in one transaction; refuses (changing nothing) while any file is invalid unless `force`, which skips invalid files. Afterwards ensures an orchestrator per agent, re-arms triggers, and emits `repo://rebuild_started`, `agents/workflows/triggers changed`, `repo://rebuild_completed` (with stats).
- **Where:** Tauri `repo_rebuild` (`force?`), `POST /api/repo/rebuild` (body optional `{"force": true}`), `services/repo_service.rs` (`rebuild`, `RepoStore::rebuild_checked`). No UI button.
- **Preconditions:** App running (desktop or headless).
- **Steps:**
  1. `POST /api/repo/rebuild` with no body.
  2. Break one agent's frontmatter; call again.
  3. Call with `{"force": true}`.
- **Expected:** 1: returns `ProjectionStats` (agents, workflows, triggers, connectors, skipped=0). 2: error "1 file(s) are invalid; fix them or rebuild with --force to skip them:" listing the path; projection unchanged. 3: succeeds with `skipped: 1`; run history unchanged.
- **Edge / negative cases:** Rebuild during an active run (run continues, history kept); rebuild while watcher is reloading.
- **Automation:** Covered by: `services/repo_service.rs` `rebuild_reconstructs_a_wiped_projection` | Layer: api | Gap: no test for the refuse-unless-force path at the HTTP layer; no UI.
- **Needs relay:** no
- **Status:** Shipped

### DT-10 — `nandi rebuild [--force]` CLI

- **What:** Rebuilds `state.db`'s projection without starting the app, prints counts, exits 0 or 1. Best run with the app closed.
- **Where:** `src-tauri/src/cli.rs` (`rebuild`), `services/repo_service.rs` (`rebuild_cli`, `rebuild_offline`); userguide section 12 "How to reset" step 1.
- **Preconditions:** App closed.
- **Steps:**
  1. `nandi rebuild`.
  2. Break a file, run `nandi rebuild`, then `nandi rebuild --force`.
  3. `nandi --force`.
- **Expected:** 1: "Rebuilt the projection: N agents, N workflows, N triggers, N connectors." exit 0. 2: "Rebuild failed: 1 file(s) are invalid..." exit 1; with `--force` succeeds and prints "Skipped 1 invalid file(s)." 3: "--force is only valid with rebuild", exit 2.
- **Edge / negative cases:** Run while the app is open (SQLite busy); missing `HOME` (creates it); a `state.db` from another schema version fails with the DT-12 message (delete it first).
- **Automation:** Covered by: `src-tauri/src/cli.rs` `parses_rebuild` | Layer: unit (parse), manual/scripted (process exit codes) | Gap: no process-level test of `rebuild_cli`.
- **Needs relay:** no
- **Status:** Shipped

### DT-11 — Repo status (counts, problems, watching)

- **What:** Reports repo root path, agent/workflow/trigger counts, every problem (kind, id, source path, error, isError) and whether the file watcher is live.
- **Where:** Tauri `repo_status`, `GET /api/repo/status`, `services/repo_service.rs` (`RepoStore::status`); shown in Settings › Backup › This computer’s copy (DT-31).
- **Preconditions:** None.
- **Steps:** `GET /api/repo/status`; then create an agent folder named `Bad_ID` with a `skill.md` and call again.
- **Expected:** `watching: true`; second call lists a problem for the invalid ID with `isError: true`.
- **Edge / negative cases:** Duplicate agent ID (local vs vendored) reported as a duplicate; dangling references (workflow naming a missing agent) reported.
- **Automation:** Covered by: `services/domain/repo.rs` `assembles_with_duplicates_and_dangling_refs_as_problems`; `LocalCopyPanel.test.tsx` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-12 — SQLite schema

- **What:** `state.db` has one schema (`SCHEMA_VERSION` 2), created on first open: every table is `CREATE TABLE IF NOT EXISTS`, `user_version` is set to 2 and default local settings are seeded. There are no migrations: a database stamped with any other non-zero version is refused at startup with "The local database is from another Nandi build (schema N, this build uses 2). Quit Nandi, delete ~/.nandi/local/state.db with its -wal and -shm files, and start it again: your agents and workflows are in ~/.nandi/repo and aren't touched." At startup, runs left running by a crash are marked failed. `nandi verify` passes the database check only on schema v2.
- **Where:** `src-tauri/src/db.rs` (`SCHEMA`, `SCHEMA_VERSION`, `init_db`, `fail_interrupted_runs`), `services/mod.rs` (`AppState::start`), `src-tauri/src/verify.rs`.
- **Preconditions:** None (fresh `NANDI_HOME`). `sqlite3` (or another SQLite client) for steps 2 and 4.
- **Steps:** 1. Start the app on an empty home. 2. Quit, then run `PRAGMA user_version` on `HOME/local/state.db`. 3. Start the app again. 4. Quit, set `PRAGMA user_version = 1`, run `nandi verify --agent <id>`, then start the app (best with `nandi --headless` from a terminal so the error is visible). 5. Delete `state.db`, `state.db-wal` and `state.db-shm` and start again.
- **Expected:** 2: `user_version` = 2 and the tables exist. 3: nothing changes and existing runs, messages and approvals stay readable; log "Marked N interrupted runs as failed" when applicable. 4: verify reports "... has schema v1, but this build of Nandi uses v2"; the app refuses to start with the "another Nandi build" message above and leaves the file alone. 5: the app starts, and agents, workflows, triggers and settings come back from the repo (run history does not).
- **Edge / negative cases:** `user_version` 0 (a new or empty file) is initialized, not refused. Moving `HOME/local` aside (DT-41) also starts clean. In the desktop app the refusal shows no dialog (see Gaps).
- **Automation:** Covered by: `src-tauri/src/db.rs` (`init_is_idempotent_and_seeds_settings`, `a_database_from_another_schema_is_refused_with_what_to_do`) | Layer: unit | Gap: no test of verify's schema check; no test of how the desktop app shows the refusal.
- **Needs relay:** no
- **Status:** Shipped

## File watcher (hand edits)

### DT-13 — Hand edits apply live

- **What:** A recursive `notify` watcher on the repo batches events until files are quiet for 500 ms (at most 2 s after the first), maps paths to entities, and reloads each through `RepoStore` with `changed_by: watcher`. Valid changes update memory + projection and emit `repo://<kind>-<action>` plus `repo://file_changed`, and `live://changed` with who made it (DT-43, DT-44); the UI invalidates git status, the stored entity and the agent definition. The app's own writes have the same hash and are no-ops. Capability files go to the capability service's handler. Policy files (`.nandi/policies/<id>.md`) are checked, not reloaded (runs read them from disk): a change emits `repo://file_changed` with `kind: policy` and `result: changed` or `invalid`, a broken one also `repo://validation_failed`, and the UI refreshes every agent's policies (PO-11).
- **Where:** `services/watcher.rs` (`spawn`, `classify`, `DEBOUNCE`, `Target::Policy`, `check_policy`), `services/policies.rs` (`check_file`), `services/repo_store.rs` (`reload`, `rescan`), `src/lib/agent-sync.ts` (`repo://file_changed` case), `src/hooks/useAgentMemory.ts`.
- **Preconditions:** App running.
- **Steps:**
  1. Create `HOME/repo/.nandi/agents/hand-made/skill.md` with valid frontmatter in an external editor.
  2. Edit its description; then delete the folder.
  3. Edit a workflow YAML and a trigger YAML by hand.
  4. With an agent page open, add, break, then delete a policy in `.nandi/policies/`.
- **Expected:** Agent appears in the sidebar within about 1 s, updates, then disappears; workflows/triggers lists refresh; `repo://agent-created/updated/deleted` and `repo://file_changed` events on the WS stream. Step 4: the agent's Permissions follow each change (PO-11); the broken policy also emits `repo://validation_failed` (`kind: policy`).
- **Edge / negative cases:**
  - `git checkout` of many files at once: a single batch, each entity reloaded once.
  - Editor swap files (`.*.swp`, `*.tmp`, `*~`), `.git/`, the app's `.nandi-tmp-` files and workflow `runs/` artifacts are ignored.
- **Automation:** Covered by: `services/watcher.rs` `classifies_gold_paths`, `classifies_marketplace_paths`, `ignores_git_temp_files_and_run_artifacts`, `edits_on_disk_reach_the_aggregate_projection_and_bus`, `policy_edits_on_disk_are_checked_and_published` (real watcher); `services/repo_store.rs` `rescans_pick_up_new_and_removed_files`; `src/lib/agent-sync.wave1.test.ts` | Layer: api (write file, watch WS) | Gap: debounce timing (one batch per burst) isn't asserted here.
- **Needs relay:** no
- **Status:** Shipped

### DT-14 — Invalid file keeps the last good version

- **What:** A file that fails to parse or validate records a problem, emits `repo://validation_failed`, and the last good version stays live in memory and SQLite. The frontend only logs a warning; the problem count shows in Settings › Backup › This computer’s copy.
- **Where:** `services/repo_store.rs` (`reload`), `services/domain/repo.rs`, `src/lib/agent-sync.ts`.
- **Preconditions:** App running with an agent.
- **Steps:** Break the YAML frontmatter of an agent's `skill.md`; send the agent a message; fix the file.
- **Expected:** Agent keeps working with the old definition; `repo://validation_failed` emitted with the path and error; status shows 1 problem; fixing the file clears it and applies the new version.
- **Edge / negative cases:** Invalid agent ID folder name (shows **Error** in the agent console until renamed).
- **Automation:** Covered by: `services/domain/repo.rs` `invalid_reloads_keep_the_last_good_version`; `services/repo_store.rs` `loads_reloads_and_keeps_the_last_good_version` | Layer: api | Gap: no visible UI toast for validation failures (only a log line and the count).
- **Needs relay:** no
- **Status:** Shipped

### DT-15 — Watcher fallback and read-through reads

- **What:** If the watcher cannot start, the app keeps working: status shows "Not watching", and agent reads (`agent`, `skill`, `list_agents`) re-check file hashes on each read, so an edited `skill.md` still applies to the next model call.
- **Where:** `services/mod.rs` (`watch_repo`, log "Not watching the repo files"), `services/repo_store.rs` (`is_watching`, read-through).
- **Preconditions:** A way to make `notify` fail (e.g. inotify watch limit on Linux) or a unit harness.
- **Steps:** Start with the watcher disabled; edit an agent's instructions; send a message.
- **Expected:** Settings › Backup › This computer’s copy badge "Not watching"; the next turn uses the edited instructions.
- **Edge / negative cases:** Watcher restarted after shutdown (`watch_repo` is re-entrant).
- **Automation:** Covered by: `services/repo_store.rs` tests (read-through) | Layer: unit | Gap: no test that forces watcher failure.
- **Needs relay:** no
- **Status:** Shipped

## Local git operations

### DT-16 — Local git status, diff and commit

- **What:** Git status of `HOME/repo` (branch, ahead/behind, files with `added|modified|deleted|renamed|untracked`), unified diff of one file vs `HEAD`, and "stage everything and commit" returning the hash. Never pushes. Only the repo at the layout root is opened.
- **Where:** Tauri `repo_git_status`, `repo_git_diff`, `repo_git_commit`; `GET /api/repo/git/status`, `GET /api/repo/git/diff?path=`, `POST /api/repo/git/commit` `{message}`; `services/repo_service.rs` (`GitService`). UI uses status only (LocalCopyPanel, RemoteSection's local-change count).
- **Preconditions:** Repo with some uncommitted change.
- **Steps:** Call status, diff on the changed path, commit with a message, status again.
- **Expected:** Status lists the file; diff shows the change; commit returns a 40-char hash; status afterwards empty.
- **Edge / negative cases:** Empty message; repo with no commits (status still works, UI tolerates git errors); path outside the repo for diff.
- **Automation:** Covered by: none found for `GitService` directly | Layer: api | Gap: no unit tests for status/diff/commit; diff/commit have no UI.
- **Needs relay:** no
- **Status:** Partial (diff and commit exposed via command/HTTP only, no UI)

## Backup remote (Settings › Backup)

### DT-17 — Backup page and empty state

- **What:** Settings › Backup shows the git remote section and a collapsible "This computer’s copy" section. With no remote: "No remote configured" empty state and the remote form.
- **Where:** `src/components/settings/sections/BackupSection.tsx`, `RemoteSection.tsx` (`EmptyRemote`), `remote-form-model.ts`, `src/hooks/useRemote.ts`.
- **Preconditions:** No remote configured.
- **Steps:** Open Settings › Backup.
- **Expected:** Empty state text "Your agents are only on this computer..."; form visible.
- **Edge / negative cases:** Status load failure shows `remote.loadFailed` alert; skeleton while pending.
- **Automation:** Covered by: `src/components/settings/BackupSection.test.tsx` (empty state) | Layer: unit (vitest) / ui-e2e | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-18 — Configure the remote (URL, credential, branch prefix)

- **What:** Saves any git URL (`https://`, `http://`, `ssh://`, `git://`, `file://`, scp-like `user@host:path`, or a local repo path) with an optional credential (None, Token, SSH key, Username + password) and a branch prefix (default `nandi`). Pushes go to `<prefix>/<slug>`. The URL/prefix/slug (no secret) go to `config.json` `remote`; a URL containing a password is refused. Onboarding has no remote step; the form lives only in Settings › Backup.
- **Where:** `src/components/settings/RemoteForm.tsx` (`RemoteFields`), `remote-form-model.ts` (`validateRemoteUrl`, `toCredential`); Tauri `configure_remote`; `PUT /api/remote/config` `{url, credential?, prefix?, slug?}`; `services/remote/mod.rs` (`configure`, `validate_git_url`).
- **Preconditions:** A reachable git repo (a local bare repo works: `git init --bare D:\tmp\backup.git`).
- **Steps:**
  1. In Settings › Backup, enter `file:///D:/tmp/backup.git` (or a path) as Repository URL, Credential None, click **Save**.
  2. Try `https://user:pass@github.com/a/b.git`.
  3. Try `not a url` and an empty URL.
- **Expected:** 1: status card appears, target branch `nandi/<slug>`; `config.json` has `remote.url`, `branchPrefix`, `userSlug`, `hasCredentials:false`. 2: "Don't put a password or token in the URL: add it as the credential instead". 3: inline "Use an https://, ssh://, or git@host:path URL." / "Enter the repository URL."; the server rejects with "'<url>' isn't a git URL: use https://…, ssh://…, user@host:path, or the path of a git repository".
- **Edge / negative cases:**
  - Prefix `main` or `master` or blank rejected (DT-23).
  - Empty credential fields while one is stored keep the stored credential.
- **Automation:** Covered by: `services/remote/mod.rs` `accepts_only_git_urls`, `configure_validates_url_slug_and_prefix` | Layer: api | Gap: no vitest for `remote-form-model.ts` validation found.
- **Needs relay:** no
- **Status:** Shipped

### DT-19 — Remote credential stored in the keychain

- **What:** The credential is sent once and stored as JSON under keychain user `nandi.remote.credential`; long tokens are chunked (`.count` + `.0..n-1`) because the Windows Credential Manager caps a blob at 2560 bytes. It is never returned to clients, never put in URLs or git config; status only says `hasCredentials` and the kind.
- **Where:** `services/remote/credentials.rs` (`save`, `load`, `clear`), `services/remote/git_ops.rs` (`callbacks`: token as password of `x-access-token`, user/pass, SSH key file, else SSH agent / credential helper).
- **Preconditions:** A private remote needing a token (e.g. GitHub PAT, 1500+ chars for chunk test).
- **Steps:** Configure with a token; inspect `config.json` and `GET /api/remote/status`; restart and push.
- **Expected:** No token in `config.json`, status, logs or `.git/config`; push works after restart.
- **Edge / negative cases:** Credential kind none clears it; unreadable entry treated as none and logged.
- **Automation:** Covered by: `services/remote/credentials.rs` `round_trips_a_short_credential`, `chunks_a_long_token_and_reassembles_it`, `a_none_kind_clears` | Layer: unit; manual on Windows for real chunking | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-20 — Remote status card

- **What:** Shows URL, Reachable/Unreachable (read live; last fetched state when offline), branch, ahead/behind counts, last push and last pull times (relative, or "Never"), and credential kind.
- **Where:** `src/components/settings/RemoteStatusCard.tsx`; Tauri `get_remote_status`; `GET /api/remote/status`; `services/remote/git_ops.rs` (`status`).
- **Preconditions:** Remote configured.
- **Steps:** Open Settings › Backup; make a local change and commit; disconnect network and reload.
- **Expected:** Ahead count increments after commit; offline shows "Unreachable" without an error page.
- **Edge / negative cases:** Remote with no branches yet (empty repo) does not crash (git2 null-slice workaround).
- **Automation:** Covered by: `services/remote/git_ops.rs` `an_unreachable_remote_reports_unreachable`; `BackupSection.test.tsx` "shows a configured remote..." | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-21 — Test connection

- **What:** Fetches the remote's branches with the stored credential and reports success or the error.
- **Where:** **Test connection** button; Tauri `test_remote_connection`; `POST /api/remote/test`.
- **Preconditions:** Remote configured.
- **Steps:** Click **Test connection**; then point the remote at a bad URL and repeat.
- **Expected:** Toast "The remote is reachable."; on failure toast "Connection failed: {error}" (`remote.testFailed`) (no credential in the message) and a `remote://error` event.
- **Edge / negative cases:** Wrong token; SSH key path that doesn't exist.
- **Automation:** Covered by: `services/remote/mod.rs` `push_and_pull_through_the_service` (indirect) | Layer: api | Gap: no explicit failure-message test.
- **Needs relay:** no
- **Status:** Shipped

### DT-22 — Back up now (push)

- **What:** After a confirmation dialog ("Back up now?" naming branch and remote), commits any uncommitted files first (so the push carries what's on disk, including NANDI.md), then pushes HEAD to `refs/heads/<prefix>/<slug>` fast-forward only. Records `lastPushAt`/`lastPushCommit` in `config.json`; emits `remote://push_completed` and `remote://status_changed`.
- **Where:** **Back up now** button; Tauri `push_to_remote(overwrite)`; `POST /api/remote/push` `{overwrite?}`; `services/remote/mod.rs` (`push`), `git_ops.rs` (`commit_local_changes`, `push`).
- **Preconditions:** Remote configured.
- **Steps:** Edit an agent; click **Back up now** › confirm; `git ls-remote <remote>`.
- **Expected:** Toast "Pushed N commit(s) to nandi/<slug>."; remote has the branch; local working tree clean; no other branch changed.
- **Edge / negative cases:** Cancel the dialog (nothing pushed); push with nothing new (0 commits); remote offline (toast `remote.pushFailed`).
- **Automation:** Covered by: `services/remote/git_ops.rs` `uncommitted_files_are_committed_before_a_push`, `push_fast_forwards_and_refuses_diverged_history_without_overwrite`; `BackupSection.test.tsx` "shows a configured remote, its branches, and pushes" | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-23 — Branch protection (never main/master)

- **What:** Every push is checked by `assert_pushable_branch`: refuses `main`, `master`, `main/...`, `master/...`, blank names, anything outside `<prefix>/`, or a prefix that is itself protected. No flag or config overrides it.
- **Where:** `services/remote/mod.rs` (`assert_pushable_branch`).
- **Preconditions:** None.
- **Steps:** `PUT /api/remote/config` with `prefix: "main"`; hand-edit `config.json` `remote.branchPrefix` to `master` and call `POST /api/remote/push`.
- **Expected:** Both refused with an error; the remote is untouched.
- **Edge / negative cases:** `slug` containing `/..`; prefix with trailing slash.
- **Automation:** Covered by: `services/remote/mod.rs` `protects_main_and_master_with_no_way_around`; `git_ops.rs` `push_refuses_protected_branches_before_touching_the_remote` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-24 — Push conflict and Overwrite

- **What:** When the remote branch has diverged, push changes nothing and the UI opens "The remote branch has changed" with a warning and **Overwrite <branch>**, which force-pushes only the user's own branch.
- **Where:** `RemoteSection.tsx` (`pushConflict` dialog), `push_to_remote(overwrite: true)`.
- **Preconditions:** Remote configured; push once, then add a different commit to that branch from another clone.
- **Steps:** Make a local change, **Back up now**, then **Overwrite**.
- **Expected:** First push returns `conflict: true` (dialog); overwrite replaces the remote branch; other branches untouched.
- **Edge / negative cases:** Cancel keeps both histories.
- **Automation:** Covered by: `BackupSection.test.tsx` "offers overwrite on a push conflict, only for the own branch"; `git_ops.rs` `push_fast_forwards_and_refuses_diverged_history_without_overwrite` | Layer: api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-25 — Restore from remote (pull)

- **What:** Pulls only the user's own target branch. If the local repo has uncommitted files, a dialog warns "N file(s) on this computer have changes that aren't backed up". Local changes are committed first; then fast-forward, else a 3-way merge. A fresh install (only the initial commit) adopts the remote's history and layout, keeping local-only files such as NANDI.md. After commits arrive, every entity is re-scanned with `changed_by: git`. Records `lastPullAt`; emits `remote://pull_completed`.
- **Where:** **Restore from remote** button; Tauri `pull_from_remote`; `POST /api/remote/pull`; `services/remote/mod.rs` (`pull`), `git_ops.rs` (`pull`, fresh-install adopt).
- **Preconditions:** Remote with the user's branch containing extra agents (e.g. pushed from a second home).
- **Steps:** On a second `NANDI_HOME`, configure the same remote and slug, click **Restore from remote**.
- **Expected:** Toast "Pulled N commit(s)."; the remote's agents/workflows appear without restart.
- **Edge / negative cases:** Missing branch on the remote (does nothing); pull with local edits (confirm dialog first); offline.
- **Automation:** Covered by: `git_ops.rs` `pull_fast_forwards`, `pull_merges_cleanly_when_changes_do_not_overlap`, `pull_of_a_missing_branch_does_nothing`; `services/remote/mod.rs` `push_and_pull_through_the_service` | Layer: api; manual (two machines) | Gap: no test of the fresh-install adopt path by name.
- **Needs relay:** no
- **Status:** Shipped

### DT-26 — Pull merge conflicts banner

- **What:** A conflicting merge is left in progress (conflicts in the index, `MERGE_HEAD` set); nothing resolved automatically. The page shows a destructive "Merge conflicts" banner with the count, a **Show conflicted files** list, a disabled **Abort merge** (tooltip "Resolve in your git client") and **Retry**. The banner also appears from status on reload.
- **Where:** `RemoteSection.tsx` (`ConflictBanner`), `PullResult.conflictedFiles`, `RemoteStatus.conflicts`.
- **Preconditions:** Same file changed differently locally and on the remote branch.
- **Steps:** **Restore from remote**; expand files; resolve with git in `HOME/repo`; **Retry**.
- **Expected:** Banner lists the conflicted paths; after resolving and retry, banner gone.
- **Edge / negative cases:** Reload the page mid-conflict (banner persists from status).
- **Automation:** Covered by: `git_ops.rs` `pull_merges_and_leaves_conflicts_unresolved`; `BackupSection.test.tsx` "shows the conflict banner with files, a disabled abort, and retry", "shows conflicts a pull reports" | Layer: api + unit | Gap: no in-app abort (by design).
- **Needs relay:** no
- **Status:** Shipped

### DT-27 — Remote branch browser

- **What:** Lists every branch on the remote, highlights the user's own ("Your branch") with an enabled **Pull into local**; other branches' button is disabled with "Only your own branch can be pulled".
- **Where:** `RemoteSection.tsx` (`BranchBrowser`); Tauri `list_remote_branches`; `GET /api/remote/branches`.
- **Preconditions:** Remote with several branches.
- **Steps:** Open Settings › Backup › "Branches on the remote"; hover a teammate's branch **Pull into local**.
- **Expected:** Sorted list, own branch badged; empty remote shows "No branches yet."
- **Edge / negative cases:** Listing fails (offline): inline error.
- **Automation:** Covered by: `BackupSection.test.tsx` "shows a configured remote, its branches, and pushes" | Layer: unit / api | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-28 — Change or remove the remote

- **What:** **Change remote** opens the form inline (Save/Cancel). **Remove remote** asks "Remove the remote?"; confirming removes the managed git remote and its tracking refs, forgets the keychain credential and the `config.json` entry; the local repo and remote repository are untouched.
- **Where:** `RemoteSection.tsx`; Tauri `configure_remote`, `clear_remote`; `DELETE /api/remote`.
- **Preconditions:** Remote configured.
- **Steps:** Change URL and save; then Remove › confirm.
- **Expected:** Toast "Remote removed. Your local repo is unchanged."; empty state returns; `git remote` in `HOME/repo` no longer lists the Nandi-managed remote (user's own `origin` untouched); keychain entry gone.
- **Edge / negative cases:** Remove when the remote was already gone (no error).
- **Automation:** Covered by: `BackupSection.test.tsx` "removes the remote after confirming", "edits the identity" | Layer: api | Gap: no test asserting the keychain entry is cleared on remove.
- **Needs relay:** no
- **Status:** Shipped

### DT-29 — Legacy `nandi/` branch notice and switch

- **What:** Settings › Backup offered to move a remote off the old `nandi/` branch prefix.
- **Needs relay:** no
- **Status:** Removed (2026-10-22, no legacy support)

### DT-30 — Remote events

- **What:** The remote service publishes `remote://status_changed`, `remote://push_completed`, `remote://pull_completed`, and `remote://error` (operation + message, never a credential) on the event bus / WS.
- **Where:** `services/remote/mod.rs`.
- **Preconditions:** WS client connected (`/ws` with token) or desktop listener.
- **Steps:** Push, pull and test against a bad URL while watching events.
- **Expected:** One event per operation in order; error events carry no secret.
- **Edge / negative cases:** Concurrent push and pull are serialized by a lock.
- **Automation:** Covered by: none specific | Layer: api | Gap: no event assertion tests.
- **Needs relay:** no
- **Status:** Shipped

### DT-31 — This computer's copy panel

- **What:** Settings › Backup › This computer’s copy shows "Local repository" with the root path, a "Watching for changes"/"Not watching" badge, "N agents · N workflows · N triggers", "N problems in the repo files (N errors)", and "Branch X · N uncommitted changes" (git line hidden if git status fails).
- **Where:** `src/components/settings/LocalCopyPanel.tsx`; `repo_status`, `repo_git_status`.
- **Preconditions:** None.
- **Steps:** Open the panel; introduce an invalid file; make an uncommitted change.
- **Expected:** Counts, problems and change count update (git status refetches on `repo://file_changed`).
- **Edge / negative cases:** Status error shows message and **Try again**.
- **Automation:** Covered by: `src/components/settings/LocalCopyPanel.test.tsx` (3 tests) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-32 — Dashboard backup nudge (cross-ref Dashboard)

- **What:** Once something has run and no remote is configured, the Dashboard shows "Back up your agents to a git remote so you don't lose them." with **Set up backup** (opens Settings › Backup) and **Not now** (snoozes 30 days, stored in webview `localStorage`).
- **Where:** `src/components/dashboard/FirstRunHints.tsx`, `dashboard-model.ts` (`showBackupNudge`, `BACKUP_SNOOZE_MS`).
- **Preconditions:** No remote; at least one finished run output.
- **Steps:** Finish a run, open Dashboard, click **Not now**, reload.
- **Expected:** Nudge shown, then hidden after Not now and across reloads.
- **Edge / negative cases:** Configure a remote: nudge never shows.
- **Automation:** Covered by: `src/components/dashboard/dashboard-model.test.ts` (`showBackupNudge`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

## Home folder and environment variables

### DT-33 — `~/.nandi` copied to `~/.nandi`

- **What:** On first launch, `~/.nandi` was copied to `~/.nandi`.
- **Needs relay:** no
- **Status:** Removed (2026-10-22, no legacy support)

### DT-34 — `NANDI_HOME`

- **What:** `NANDI_HOME` replaces `~/.nandi` (leading `~` expanded) so separate instances can run side by side; single-instance locking is skipped when a custom home is set. An empty value counts as unset.
- **Where:** `utils/nandi_config.rs` (`HOME_ENV`, `has_custom_home`), `utils/env.rs`, `src-tauri/src/lib.rs` (single-instance guard).
- **Preconditions:** None.
- **Steps:** Start two instances with different `NANDI_HOME` and `--headless --port` values; then start one with `NANDI_HOME=` (empty).
- **Expected:** Two independent homes and databases; the empty value uses `~/.nandi`.
- **Edge / negative cases:** Keyring entries are shared across homes (same service name) — keys saved in one home appear in the other.
- **Automation:** Covered by: `utils/nandi_config.rs` `nandi_home_replaces_the_default`; `utils/env.rs` `an_empty_value_counts_as_unset` | Layer: unit; manual for two instances | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-35 — `NANDI_*` environment variables

- **What:** Every env var the app reads is `NANDI_*` (e.g. `NANDI_HOME`, `NANDI_RELAY_URL`, `NANDI_LLM_PROVIDER`, `NANDI_LLM_MODEL`). An empty value counts as unset. `NANDI_*` names are not read. Variables set for agent containers (`NANDI_AGENT_ID`, `NANDI_LLM_PROXY_TOKEN`, ...; SB-11) also use only the `NANDI_` names. There is no MCP token since #9 (RT-06).
- **Where:** `utils/env.rs` (`var`, `var_os`), `services/config/repo_config.rs` (`PROVIDER_ENV`, `MODEL_ENV`), `src-tauri/src/runtime.rs` (container env), `runtimes/nandi-runtime/app.py`.
- **Preconditions:** None.
- **Steps:** 1. Start with `NANDI_RELAY_URL=ws://127.0.0.1:8787/relay`; inspect a container's env (`docker inspect nandi-agent-<tag>-<id>`). 2. Start with only `NANDI_RELAY_URL` set.
- **Expected:** 1: relay URL honoured (RL-04); the container has `NANDI_*` variables and no `NANDI_*` ones. 2: the variable is ignored.
- **Edge / negative cases:** `NANDI_RELAY_URL=` (empty) behaves as unset.
- **Automation:** Covered by: `utils/env.rs` `an_empty_value_counts_as_unset` | Layer: unit | Gap: no test of the container env names.
- **Needs relay:** no
- **Status:** Shipped

### DT-36 — Pre-v6 install migrated to the gold-copy layout

- **What:** A pre-v6 install was migrated to the gold-copy layout at startup.
- **Needs relay:** no
- **Status:** Removed (2026-10-22, no legacy support)

### DT-37 — Legacy trigger rows: secrets to keyring, rules to files

- **What:** Trigger rows from an older `state.db` moved their secrets to the keyring and got repo files.
- **Needs relay:** no
- **Status:** Removed (2026-10-22, no legacy support)

### DT-38 — `NANDI.md` renamed to `NANDI.md`

- **What:** A repo's `NANDI.md` was renamed to `NANDI.md`.
- **Needs relay:** no
- **Status:** Removed (2026-10-22, no legacy support)

## Configuration and data locations

### DT-39 — Enterprise LLM config (`.nandi/llm.yaml`)

- **Duplicate:** tested under MD-18. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### DT-40 — Local config file `~/.nandi/config.json`

- **What:** Non-secret machine settings: `relay_url`, `remote` (URL, prefix, slug, credential kind, has-credentials, last push/pull), `mascot_name`, plus unknown keys preserved on save. Written atomically (temp + rename). Invalid JSON is an error, a missing file is defaults.
- **Where:** `utils/nandi_config.rs` (`NandiConfig`, `load_from`, `save_to`, `update_in`).
- **Preconditions:** None.
- **Steps:** Configure a remote and rename the bull; open `config.json`; add an unknown key; change a setting; reopen.
- **Expected:** Values present, no secrets, unknown key survives.
- **Edge / negative cases:** Corrupt JSON → error reported where the config is read.
- **Automation:** Covered by: `utils/nandi_config.rs` `missing_file_yields_defaults`, `round_trips_and_preserves_unknown_keys`, `relay_url_uses_its_snake_case_key`, `rejects_invalid_json` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-41 — Data locations and reset procedure (user guide section 12)

- **What:** Documented locations: repo `HOME/repo/.nandi/` (git); runtime `HOME/local/state.db`; audit logs `HOME/local/logs/` (`llm.log`, `egress.log`, `triggers.log`, `pilots.log`, `connectors.log`, `delegations.log`, `verify.log`); workdirs `HOME/local/work/<id>/`; trash `HOME/trash/`; caches `HOME/local/cache/`; app log (macOS `~/Library/Logs/<app id>/`); Windows release `HOME/server.log` (access link). Reset ladder: 1) `nandi rebuild [--force]`, 2) restart a container, 3) move `HOME/local` aside, 4) move all of `HOME` aside (setup starts again; restore agents from the backup remote).
- **Where:** `docs/userguide/userguide.md` §12; `docs/developer/data-model.md`; `utils/nandi_config.rs` (`local_dir`, `state_db_path`, `logs_dir`, `work_dir`).
- **Preconditions:** An install with agents and history.
- **Steps:** Perform reset 3: quit, rename `HOME/local`, start.
- **Expected:** All agents/workflows/triggers/settings come back from the repo; run history, approvals and logs are empty; a new `state.db` is created.
- **Edge / negative cases:** Reset 4: app starts at first-run; keyring entries survive unless deleted.
- **Automation:** Covered by: `services/repo_service.rs` `a_fresh_database_is_rebuilt_from_files_on_load` (reset 3 core) | Layer: manual | Gap: userguide step 4 mentions "sign-in screen and clones your agents repo again", which reflects the pre-remote GitHub flow (see Gaps).
- **Needs relay:** no
- **Status:** Shipped

### DT-42 — Snapshot read APIs (cross-ref Agents/Runs/Dashboard)

- **What:** The UI reads state only through snapshots and applies `agent://*` / `dashboard://*` events on top: agent snapshot, conversation, Ask conversations, workflows, run detail, dashboard snapshot, and resolving/opening files a run touched (open with default app or reveal in file manager; "That file isn't on this computer anymore" when gone).
- **Where:** `src-tauri/src/commands/snapshots.rs` (`get_agent_snapshot`, `get_conversation`, `get_studio_conversations`, `get_workflows`, `get_run_detail`, `resolve_run_file`, `open_run_file`, `get_dashboard_snapshot`); `services/snapshots.rs`; `GET /api/snapshots/agent/{id}`, `GET /api/snapshots/dashboard`, `GET /api/runs/{id}`, `POST /api/runs/{id}/file`, `GET /api/conversations/{id}`.
- **Preconditions:** Some runs.
- **Steps:** Call each route; for `runs/{id}/file` pass a path outside the workdir.
- **Expected:** JSON snapshots; path outside the workdir returns null (no traversal).
- **Edge / negative cases:** Unknown IDs; deleted workdir.
- **Automation:** Covered by: `services/snapshots.rs` tests (workdir_file) | Layer: api | Gap: detailed behaviour belongs to the owning sections.
- **Needs relay:** no
- **Status:** Shipped

### DT-43 — Live UI: notes and work folders are watched

- **What:** Besides the repo (DT-13), Nandi watches My notes (AG-28) and every agent's work folder (`~/.nandi/local/work/<agent>/`). A note added, edited or deleted outside the app, or by an agent, shows on My notes within about a second, with no reload. The repo watcher also reports `installed.yaml` (installed packs) and `NANDI.md` (your profile). Every change is one `live://changed` event, and the screens refetch exactly what it affects: the note and the list, the agent's definition and policies, the workflow, the trigger, preferences for `settings.yaml`, connectors for `connectors.yaml`, the Library for packs, Settings › Account for `NANDI.md`.
- **Where:** `services/live/` (`folders.rs` `FolderWatcher`, `watchable`; `mod.rs` `ChangeFeed`, `StoreChanges`, `LiveService::watch_folders`), `services/watcher.rs` (`Target::Packs`, `Target::Profile`), `services/notes.rs` (`watch`), `src/lib/live-changes.ts` (`queryKeysFor`, `applyLiveChange`), `src/lib/agent-sync.ts` (`live://changed`). Event `live://changed` (`LiveChange`).
- **Preconditions:** App running; My notes opened once (or the first launch done).
- **Steps:**
  1. Open My notes on a note. Edit the same file in Notepad and save.
  2. Add a new `.md` file to the notes folder in Explorer.
  3. Edit `.nandi/connectors.yaml` or `.nandi/settings.yaml` by hand with Library or Settings open.
  4. Save a file into `~/.nandi/local/work/<agent>/` while that agent's page is open.
- **Expected:** Step 1: the note's text changes within about a second, "Updated just now outside Nandi". Step 2: the new note appears in the list. Step 3: the open screen shows the new values. Step 4: Home's Recent changes lists the file under that agent.
- **Edge / negative cases:**
  - Editor temp files (`.x.md.swp`, `x.md~`, `x.md.tmp`), hidden files and `.git/` are ignored; so are `node_modules`, `target`, `__pycache__`, `.venv`, `venv`, `.cache` and `dist` in work folders.
  - A notes folder shared as a drive root or the home folder isn't watched (log: "Not watching …"); Refresh still works (DT-46).
  - A note being edited keeps the unsaved text when the file changes underneath (see the task's follow-ups).
- **Automation:** Covered by: `live/folders.rs` `classifies_notes_and_work_files`, `changes_in_real_folders_arrive_batched_by_area`, `the_home_folder_and_drive_roots_are_never_watched`; `live/tests.rs` `the_repo_watcher_reports_profile_and_pack_files`; `watcher.rs` `classifies_the_profile_and_installed_packs`; `src/lib/live-changes.test.ts` | Layer: unit (real folders) | Gap: none in automation; the Notepad check is manual.
- **Needs relay:** no
- **Status:** Shipped

### DT-44 — Who changed it: "Updated just now by …"

- **What:** Each change names who made it: **by you** (the app, on your command), **by** an agent (its run was going when the file changed), **by the** … **workflow** (that run was a workflow step), or **outside Nandi** (an editor, a script, a `git pull`). An open note and an agent's page show "Updated just now by Chief of Staff"; with no change this session, a note shows its file time.
- **Where:** `services/live/author.rs` (`writer`, `active_runs`, `author_of`), `services/live/own_writes.rs` (the app's own writes, noted by `fs_atomic` and `NotesFolder`), `src/components/live/UpdatedBy.tsx`, `live-model.ts` (`authorText`, `updatedText`), `MyNotesView.tsx`, `AgentView.tsx`.
- **Preconditions:** App running; Chief of Staff may keep notes (AG-28).
- **Steps:**
  1. Save a note on the My notes page.
  2. Edit it in Notepad.
  3. Ask Chief of Staff to note something.
  4. Run a workflow whose agent writes a file in its work folder.
- **Expected:** 1: "Updated just now by you". 2: "… outside Nandi". 3: "… by Chief of Staff". 4: Recent changes says "by the <workflow> workflow".
- **Edge / negative cases:**
  - With the notes share read-only, a change during a run is still "outside Nandi" (agents can't write there).
  - Two agents running at once: a note goes to Chief of Staff first, else the latest run.
  - Changes made while the app was closed have no author.
- **Automation:** Covered by: `live/tests.rs` `notes_say_who_changed_them`, `files_in_a_work_folder_name_the_agent_or_workflow_running`, `the_apps_own_definition_writes_are_by_you`, `active_runs_are_running_or_just_ended`; `live/author.rs` tests; `live/own_writes.rs` tests; `live-model.test.ts`; `MyNotesView.test.tsx` `says who changed the open note` | Layer: unit | Gap: the author is the run going at the time, not the exact writer (see `docs/developer/live-ui.md`).
- **Needs relay:** no
- **Status:** Shipped

### DT-45 — Recent changes on Home

- **What:** Home lists the latest 8 file changes (Recent changes): "Changed deadlines.md", "Added Analyst" with the files, who and when. Each row opens its page: the note, the agent, the workflow, the trigger, the Library or Settings. Hidden until something changes; the core keeps the last 200 this session.
- **Where:** `src/components/live/RecentChanges.tsx`, `src/hooks/useLiveChanges.ts` (`useRecentChanges`), `services/live/mod.rs` (`ChangeFeed::recent`), command `list_recent_changes`, `GET /api/changes`.
- **Preconditions:** App running.
- **Steps:** Edit a note outside Nandi, save an agent in Studio, add a file to an agent's work folder; open Home.
- **Expected:** Three rows, newest first, each with its author; clicking one opens its page.
- **Edge / negative cases:** A restart empties the list (it's in memory).
- **Automation:** Covered by: `RecentChanges.test.tsx`, `live/tests.rs` `the_recent_list_keeps_the_latest_changes_newest_first`, `live-changes.test.ts` | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-46 — Refresh, F5 and Ctrl+R

- **What:** A **Refresh** button on My notes, an agent's page, Workflows, Triggers and Home's Recent changes, F5 and Ctrl/Cmd+R anywhere in the main window, and **Refresh** in the command palette re-read every definition from disk and refetch every file-backed screen. It works with no watcher at all (a network drive, a laptop that slept). F5 no longer reloads the webview.
- **Where:** `services/live/mod.rs` (`refresh_from_disk`), command `refresh_from_disk`, `POST /api/changes/refresh`, `src/lib/live-changes.ts` (`refreshFromDisk`, `FILE_BACKED_KEYS`), `src/components/live/RefreshButton.tsx`, `src/lib/commands/live-commands.ts`, `src/hooks/use-keyboard-shortcuts.ts`.
- **Preconditions:** App running.
- **Steps:**
  1. Start with the watcher unable to see a change (for example `NANDI_HOME` on a network share), or just press F5 after an edit.
  2. Edit a skill.md or a note outside Nandi.
  3. Press F5, or click Refresh.
- **Expected:** The screen shows the edit; the page doesn't reload (no flash, the open tab and scroll stay).
- **Edge / negative cases:** A refresh that fails shows the error as a toast and refetches nothing.
- **Automation:** Covered by: `live/tests.rs` `refresh_works_with_the_watcher_off`; `live-changes.test.ts` (`Refresh re-reads the disk, then every file-backed screen`); `use-keyboard-shortcuts.test.ts` (`F5 and Cmd/Ctrl+R refresh from disk instead of reloading`) | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### DT-47 — Re-scan when the OS drops file events

- **What:** When the OS may have lost file events (it sets the rescan flag, the watcher reports an error, or more than 512 files change in one batch, as a big `git checkout` does), the watcher re-reads that whole area (every definition, policy, capability, `installed.yaml` and `NANDI.md`; or the whole notes or work folder) and logs `Re-scanning <folder>: <why>`.
- **Where:** `services/watcher.rs` (`signals`, `Batch::add`, `BURST_LIMIT`, `targets_of`), `services/live/folders.rs` (`FolderClassifier::changes`).
- **Preconditions:** App running.
- **Steps:** Check out a branch that changes hundreds of files in the repo, or copy 600 files into the notes folder at once.
- **Expected:** The log shows one "Re-scanning …" line; the screens end up current.
- **Edge / negative cases:** A Windows change buffer overflow is silent; the burst rule covers it.
- **Automation:** Covered by: `watcher.rs` `events_the_os_may_have_dropped_rescan_everything`; `live/folders.rs` `a_batch_that_lost_events_rescans_every_area` | Layer: unit | Gap: no test makes the OS really drop events.
- **Needs relay:** no
- **Status:** Shipped

## Platform: CLI, headless, OS specifics

### PL-01 — CLI usage, help and argument validation

- **What:** `nandi -h|--help` prints usage (options, commands, `NANDI_RELAY_URL`, `NANDI_HOME`) and exits 0. Unknown flags exit 2 with "Unknown option '<x>'. Run with --help for usage." Positional args (OS file paths) and macOS `-psn_*` are ignored. `--port` accepts `--port N` or `--port=N`.
- **Where:** `src-tauri/src/cli.rs` (`USAGE`, `CliOptions::parse`), `src-tauri/src/lib.rs` (`run`).
- **Preconditions:** Built binary (`cargo build --release --bin nandi`).
- **Steps:** `nandi --help`; `nandi --foo`; `nandi --port x`; `nandi --port`; `nandi file.txt` .
- **Expected:** Usage text; "Unknown option '--foo'..." exit 2; "Invalid --port value 'x'"; "--port needs a value"; positional ignored (app starts).
- **Edge / negative cases:** Subcommands only recognised as the first argument (`nandi --server verify` is not verify).
- **Automation:** Covered by: `cli.rs` `parses_help`, `rejects_unknown_flags_and_bad_ports`, `ignores_positional_args_and_macos_psn`, `usage_documents_the_relay_url_override` | Layer: unit | Gap: no process exit-code test.
- **Needs relay:** no
- **Status:** Shipped

### PL-02 — `--server` and `--port`

- **What:** The desktop app always serves the web UI/API on `127.0.0.1:<random>`. `--server` binds `0.0.0.0` on port 17463 (or `--port`) and prints the access link `http://localhost:<port>/?token=<token>` plus a warning that anyone on the network with the link can connect. `--port` alone keeps loopback.
- **Where:** `cli.rs` (`bind`, `announce`, `access_url`), `http::serve`; token persisted at `HOME/web_token`.
- **Preconditions:** None.
- **Steps:** `nandi --server`; open the link from another machine; `nandi --port 9000`.
- **Expected:** Listening on 0.0.0.0:17463 with link printed; second binds 127.0.0.1:9000.
- **Edge / negative cases:** Port already in use (startup error); request without token rejected.
- **Automation:** Covered by: `cli.rs` `defaults_to_a_random_loopback_port`, `server_binds_all_interfaces`, `builds_access_links` | Layer: unit; api for the served server | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### PL-03 — Headless mode

- **What:** `nandi --headless` starts the core (same app data dir as the desktop app), the egress proxy and the web server without any window, on `127.0.0.1:17463` unless `--server`/`--port`; prints "<Name> is up. N agents ready." and the access link; logs to stderr; stops on Ctrl-C (orchestrators shut down, runs resume next launch).
- **Where:** `src-tauri/src/headless.rs`, `cli.rs`.
- **Preconditions:** None (Docker for running agents).
- **Steps:** `NANDI_HOME=<tmp> nandi --headless --port 17611`; open the link; Ctrl-C.
- **Expected:** Link printed; web UI works; clean shutdown log "Shutting down".
- **Edge / negative cases:** `--headless --server` prints the all-interfaces warning; two headless instances with different homes/ports run together.
- **Automation:** Covered by: `cli.rs` `headless_stays_on_loopback_unless_server` | Layer: api (primary harness for automated testing) | Gap: no automated boot test of the headless binary.
- **Needs relay:** no
- **Status:** Shipped

### PL-04 — Windows console attach and `server.log`

- **What:** Windows release builds are GUI-subsystem apps; when started with arguments from a terminal they attach to the parent console so `--help`, errors and links print. The access link is also written to `HOME/server.log` on Windows release builds.
- **Where:** `cli.rs` (`attach_parent_console`, `record_access_url`), `main.rs` (`windows_subsystem = "windows"`).
- **Preconditions:** Windows, release build.
- **Steps:** From PowerShell, `nandi.exe --help`; `nandi.exe --headless`; open `HOME/server.log`.
- **Expected:** Usage printed in the same console; link in `server.log`.
- **Edge / negative cases:** Launched from Explorer: no console, no error.
- **Automation:** Covered by: none | Layer: manual | Gap: Windows-only, release-only code.
- **Needs relay:** no
- **Status:** Shipped

### PL-05 — Other CLI commands (cross-references)

- **What:** `nandi verify --agent <id>` (prerequisite check, writes `verify.log`; see Agents/Runtime section), `nandi keygen [--key]` and `nandi sign --kind agent|pack --id --release (--commit|--repo) [--key]` (see Marketplace), `nandi name` / `nandi rename <NAME>|--reset` (see Appearance/mascot). Each runs and exits without opening a window.
- **Where:** `cli.rs`, `lib.rs`, `verify.rs`, `services/marketplace/signature.rs`, `cli_mascot.rs`. (The relay is `nandi-relay` from vaultdbai/nandi-relay, RL-07; the app has no `relay` command.)
- **Preconditions:** Per command.
- **Steps:** `nandi verify` (no agent), `nandi --agent a`, `nandi sign --kind tool`.
- **Expected:** "verify needs --agent <id>"; "--agent is only valid with verify"; "--kind must be agent or pack".
- **Edge / negative cases:** `sign` with both or neither of `--commit`/`--repo` → "exactly one".
- **Automation:** Covered by: `cli.rs` `parses_verify`, `parses_publisher_commands`, `parses_relay`; `cli_mascot.rs` tests | Layer: unit | Gap: see owning sections.
- **Needs relay:** no
- **Status:** Shipped

### PL-06 — Single instance and window state

- **Duplicate:** tested under SH-02, SH-03. This ID is kept so old references stay valid; don't test it separately.
- **Needs relay:** no

### PL-07 — Per-OS window and title bar

- **What:** Per-platform Tauri configs: macOS uses no decorations, transparent window with `hudWindow` vibrancy and custom traffic-light controls (`macOSPrivateApi`); Windows no decorations with custom window controls; Linux native decorations and `LinuxTitleBar`. Min size 1000x700. Platform strings and shortcut formatting (Cmd vs Ctrl) via `getPlatformStrings` / `use-platform`. The title bar hosts the Nandi bull (state from the app) and the Update button.
- **Where:** `src-tauri/tauri.conf.json`, `tauri.macos.conf.json`, `tauri.windows.conf.json`, `tauri.linux.conf.json`; `src/components/titlebar/*` (`MacOSWindowControls.tsx`, `WindowsWindowControls.tsx`, `LinuxTitleBar.tsx`); `src/lib/platform-strings.ts`; `src/hooks/use-platform.ts`; `docs/developer/cross-platform.md`.
- **Preconditions:** One machine per OS.
- **Steps:** On each OS: minimize/maximize/close from the title bar; drag the window by the title bar; check shortcut hints in menus.
- **Expected:** Controls work; drag regions work; macOS shows Cmd glyphs, others Ctrl.
- **Edge / negative cases:** Resize below 1000x700 refused.
- **Automation:** Covered by: `src/hooks/use-platform.test.ts`; `utils/platform.rs` tests | Layer: manual (per OS) | Gap: no cross-OS e2e.
- **Needs relay:** no
- **Status:** Shipped

### PL-08 — In-app auto-updater

- **What:** Desktop-only. Checks at startup and every 6 hours against `https://github.com/vaultdbai/nandi/releases/latest/download/latest.json` (the release workflow attaches it to every GitHub release) (fallback `https://vaultdb.ai/...`); the signature is verified against the bundled public key. When a newer version exists the title bar shows **Update to <version>**; one click downloads (with percent), installs (Windows `passive` mode) and relaunches. Failed background checks are silent and never hide a found update; a failed install shows a retry. App menu **Check for Updates** reports "Up to date", "available", or a check-failed notification.
- **Where:** `src/store/update-store.ts` (`startUpdateChecks`, `checkForUpdate`, `installUpdate`, `UPDATE_CHECK_INTERVAL_MS`), `src/components/titlebar/UpdateButton.tsx`, `src/lib/menu.ts` (`check-updates`), `src/App.tsx`, `tauri.conf.json` `plugins.updater`, `lib.rs` (`tauri_plugin_updater`, `tauri_plugin_process`).
- **Preconditions:** An installed older version and a published newer release with `latest.json` at the endpoint.
- **Steps:** Launch the older build; wait or use **Check for Updates**; click **Update to X**.
- **Expected:** Button appears; progress; app restarts on the new version.
- **Edge / negative cases:** Offline (silent; menu check shows failure); tampered artifact (rejected by signature); web (non-desktop) build never checks.
- **Automation:** Covered by: `src/store/update-store.test.ts`, `src/components/titlebar/UpdateButton.test.tsx` | Layer: unit; manual for a real update | Gap: endpoint publication not automated (see Gaps).
- **Needs relay:** no
- **Status:** Shipped (2026-11-03: the endpoint is the `latest.json` the release workflow attaches to each GitHub release; the S3 copy to vaultdb.ai is optional)

### PL-09 — Release workflow (GitHub Actions)

- **What:** On a `v*` tag or manual dispatch, builds macOS (`app,dmg`), Windows (`msi`) and Linux (`appimage`, Ubuntu 22.04) with `tauri-action`, creates a draft GitHub release "Nandi vX" with updater JSON and signatures, injects `NANDI_GITHUB_CLIENT_ID` and `NANDI_TRIAL_DEEPSEEK_KEY`, and (when S3 secrets are set) uploads each installer to `s3://<bucket>/downloads/nandi/<version>/` and a stable `downloads/nandi/latest/VaultDB-Nandi-<os>.<ext>` plus `latest/VERSION`.
- **Where:** `.github/workflows/release.yml`; `docs/developer/releases.md`.
- **Preconditions:** Repo secrets `TAURI_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, optional `S3_ID`, `S3_KEY`, `AWS_REGION`, `AWS_BUCKET`, `NANDI_GITHUB_CLIENT_ID`, `NANDI_TRIAL_DEEPSEEK_KEY`.
- **Steps:** Push tag `v0.1.1` (or dispatch with a version).
- **Expected:** Draft release with dmg/msi/AppImage, `.sig` files and `latest.json`; S3 objects when enabled; step fails if not exactly one installer per bundle.
- **Edge / negative cases:** Forks without S3 secrets still release to GitHub.
- **Automation:** Covered by: the workflow itself | Layer: manual (CI) | Gap: stable download names still say "VaultDB-Nandi"; `bundle.publisher`, descriptions and copyright in `tauri.conf.json` are template placeholders ("Your Name", "A Tauri React template application").
- **Needs relay:** no
- **Status:** Partial (template metadata and old download names remain)

### PL-10 — Release preparation script

- **What:** `pnpm run release:prepare vX.Y.Z` checks a clean git tree, runs `pnpm run check:all`, bumps versions in `package.json`, `src-tauri/Cargo.toml` and `tauri.conf.json`, runs `pnpm install`, verifies updater artifacts and pubkey are configured, runs `cargo check` in `src-tauri`, then prints the git commands to tag and push (asks before running them).
- **Where:** `scripts/prepare-release.js`; `package.json` `release:prepare`.
- **Preconditions:** Clean tree, toolchain installed.
- **Steps:** `pnpm run release:prepare v0.1.1`; then with a dirty tree; then `pnpm run release:prepare 1.0`.
- **Expected:** Versions bumped; dirty tree aborts listing changes; bad version prints usage and exits 1.
- **Edge / negative cases:** Works the same from PowerShell and bash (no shell builtins; `cargo` must be on `PATH`). The usage text names `pnpm run release:prepare`.
- **Automation:** Covered by: none | Layer: manual | Gap: manual.
- **Needs relay:** no
- **Status:** Shipped

### PL-11 — Product and binary naming (Nandi rename)

- **What:** `productName` "Nandi" (installer name and install folder, so it installs beside an old "VaultDB Nandi"); window titles "Nandi"; the app binary is `nandi` (the package's only binary; the relay, `nandi-relay`, is built in vaultdbai/nandi-relay); bull icons; UI copy in all four locales says "Nandi". The bundle identifier is `com.nandi.app` (app data folder and keychain service); nothing is carried over from an old "VaultDB Nandi" install. Unchanged on purpose: `nandi://`/`repo://` event names, `nandi.*` keychain entry names, Cargo/npm package `tauri-app`, the `.nandi/` repo folder.
- **Where:** `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` (`[[bin]] nandi`), `src-tauri/icons/`, `locales/*.json`.
- **Preconditions:** Release build.
- **Steps:** Install; check Start menu / Applications name, window title, `nandi --help` header.
- **Expected:** "Nandi" everywhere user-visible; a previous "VaultDB Nandi" install stays until uninstalled, and its data is not imported (no legacy support).
- **Edge / negative cases:** Old `nandi-relay` name (see Gaps).
- **Automation:** Covered by: `src/themes/contrast.test.ts` (bull colours) | Layer: manual | Gap: no string audit test for leftover "Nandi".
- **Needs relay:** no
- **Status:** Shipped

### PL-12 — Website rebrand (external repo)

- **What:** The vaultdb.ai site was rebranded to Nandi (name, logo, metadata, theme colours matched to the app, animated bull hero, copy, OG image). The site code lives in `D:\Aayushi\vaultdb-web`, not this repo.
- **Where:** `docs/tasks-done/task-2026-10-19-website-rebrand-nandi.md`.
- **Preconditions:** Access to the deployed site.
- **Steps:** Open vaultdb.ai in light and dark mode; check title, favicon, OG tags, hero animation with `prefers-reduced-motion`.
- **Expected:** Nandi branding throughout; no hidden nickname anywhere in shipped code.
- **Edge / negative cases:** Download links point at `downloads/nandi/latest/VaultDB-Nandi-*` (old names).
- **Automation:** Covered by: none here | Layer: manual | Gap: out of this repo.
- **Needs relay:** no
- **Status:** Shipped (external)

## Developer quality gates

### QA-01 — `pnpm run check:all`

- **What:** One gate running, in order: `typecheck` (tsc), `lint` (ESLint, max 0 warnings), `ast:lint` (ast-grep), `format:check` (Prettier), `rust:fmt:check`, `rust:clippy` (`-D warnings`), `test:run` (Vitest), `rust:test` (cargo test). `fix:all` applies lint/format/clippy fixes.
- **Where:** `package.json` scripts; `docs/developer/static-analysis.md`, `testing.md`.
- **Preconditions:** `pnpm install`; Rust toolchain per `rust-toolchain.toml` (pinned to 1.98.1, so local clippy matches CI).
- **Steps:** `pnpm run check:all`.
- **Expected:** Exit 0, every step green.
- **Edge / negative cases:** Introduce a Zustand destructure: `ast:lint` fails.
- **Automation:** Covered by: itself, and `.github/workflows/ci.yml` (ubuntu-latest, on pull requests and pushes to main: the same gates, with `pnpm run build` before clippy; Rust from `rust-toolchain.toml`; `#[ignore]` Docker, network and live tests skipped) | Layer: unit | Gap: CI runs Linux only.
- **Needs relay:** no
- **Status:** Shipped

### QA-02 — ast-grep architecture rules

- **What:** Four error-level rules over `src/**/*.ts(x)`: `no-destructure-zustand`, `hooks-in-hooks-dir`, `no-store-subscription-in-lib`, `no-raw-palette-colors`; and two over `src-tauri/src/**/*.rs`: `relay-connection-only-in-client` (only `services/relay/client.rs` opens a WebSocket or builds `RelayMessage`) and `relay-client-built-only-in-relay` (`WsRelayClient::new`/`with_options` only in `services/relay/`). Test modules are exempt. `pnpm run ast:fix` applies fixes.
- **Where:** `sgconfig.yml`, `.ast-grep/rules/{zustand,architecture,theme,relay}/*.yml`; `docs/developer/writing-ast-grep-rules.md`.
- **Preconditions:** None.
- **Steps:** Add `const { x } = useUIStore()` to a component; run `pnpm run ast:lint`.
- **Expected:** Error from `no-destructure-zustand` with its message.
- **Edge / negative cases:** Raw Tailwind palette class (e.g. `text-red-500`) flagged. `tokio_tungstenite::connect_async(…)` in `services/a2a/` flagged; the same inside `mod tests` isn't.
- **Automation:** Covered by: none (rule tests) | Layer: unit | Gap: the rules have no rule tests (add a `testConfigs` entry to `sgconfig.yml` with the first one).
- **Needs relay:** no
- **Status:** Shipped

### QA-03 — knip and jscpd (periodic cleanup)

- **What:** `pnpm run knip` finds unused files/exports/deps (entries `src/main.tsx`, `src/companion-main.tsx`; ignores `components/ui`, `bindings.ts`, tests). `pnpm run jscpd` finds duplicates in `ts/tsx/rs` (min 10 lines / 50 tokens) and writes `jscpd-report/`. Not part of `check:all`; driven by the `/cleanup` skill and `cleanup-analyzer` agent.
- **Where:** `knip.json`, `.jscpd.json`, `.claude` skills.
- **Preconditions:** None.
- **Steps:** Run both.
- **Expected:** Reports produced; no crash.
- **Edge / negative cases:** None.
- **Automation:** Covered by: n/a | Layer: unit | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### QA-04 — TypeScript bindings generation (tauri-specta)

- **What:** Debug builds regenerate `src/lib/bindings.ts` from Rust commands/types/events on every start; `pnpm run rust:bindings` (ignored test `export_bindings`) does it without the app. A vitest guard fails if two Rust types export under the same name (which would break Vite while `@ts-nocheck` hides it from tsc).
- **Where:** `src-tauri/src/bindings.rs` (`generate_bindings`, `export_ts_bindings`), `src-tauri/src/lib.rs`, `src/lib/bindings.test.ts`; `docs/developer/tauri-commands.md`.
- **Preconditions:** Rust toolchain.
- **Steps:** Add a command, run `pnpm run rust:bindings`, `git diff src/lib/bindings.ts`, `pnpm test:run src/lib/bindings.test.ts`.
- **Expected:** New typed command in `commands`; duplicate-name test passes.
- **Edge / negative cases:** Command not registered in `generate_bindings` → missing in TS (typecheck fails on use).
- **Automation:** Covered by: `src/lib/bindings.test.ts` "export every type name once" | Layer: unit | Gap: no check that committed bindings match the Rust source (stale bindings undetected in CI).
- **Needs relay:** no
- **Status:** Shipped

### QA-05 — E2E smoke prerequisite script (Windows)

- **What:** Read-only PowerShell check: Docker running, runtime image present (default `ghcr.io/vaultdbai/nandi-runtime:sandbox-6`), state DB schema, DeepSeek key (`nandi.llm.deepseek`) in Windows Credential Manager under `com.nandi.app`; prints pass/fail per item and the count missing. It reads `NANDI_HOME` (else `~/.nandi`) and checks `local/state.db`.
- **Where:** `scripts/e2e-smoke.ps1`; `docs/developer/e2e-checklist.md`.
- **Preconditions:** Windows, Docker.
- **Steps:** `powershell -ExecutionPolicy Bypass -File scripts/e2e-smoke.ps1`.
- **Expected:** One line per check; non-zero failures counted. Today the schema check fails on a current database: the script wants `user_version` 18 or later (`$MinSchemaVersion = 18`) and says "Launch Nandi once to migrate", but the schema is now v2 with no migrations (DT-12).
- **Edge / negative cases:** `-Image my-image:tag`.
- **Automation:** Covered by: itself | Layer: manual | Gap: only checks a DeepSeek key; ignores other providers, the Claude sign-in and the free trial.
- **Needs relay:** no
- **Status:** Known issue (`scripts/e2e-smoke.ps1` still expects schema v18+; with schema v2 its database check always fails)

### QA-06 — Test fixtures (`test-fixtures/echo-agent`)

- **What:** A minimal agent (`deepseek:deepseek-flash`, `ghcr.io/vaultdbai/nandi-runtime:sandbox-6`, `allowed-tools: [shell]`) used by the agent-parsing unit tests and by the manual E2E checklist.
- **Where:** `test-fixtures/echo-agent/skill.md`; included by `src-tauri/src/services/agents.rs` tests (`include_str!`).
- **Preconditions:** None (DeepSeek key + Docker for manual use).
- **Steps:** Copy to `HOME/repo/.nandi/agents/echo-agent/skill.md`; `nandi verify --agent echo-agent`.
- **Expected:** Agent appears live; verify passes when prerequisites are met.
- **Edge / negative cases:** None.
- **Automation:** Covered by: `services/agents.rs` tests | Layer: unit | Gap: none (`e2e-checklist.md` now copies it to `$HOME\.nandi\repo\.nandi\agents\echo-agent\`).
- **Needs relay:** no
- **Status:** Shipped

### QA-07 — Real-install upgrade check

- **What:** An ignored cargo test ran the Nandi-to-Nandi upgrade on a copy of a real home folder.
- **Needs relay:** no
- **Status:** Removed (2026-10-22, no legacy support)

### QA-08 — Task and helper scripts

- **What:** `pnpm task:complete <name|number>` moves `docs/tasks-todo/task-N-x.md` to `docs/tasks-done/task-YYYY-MM-DD-x.md`; `pnpm task:rename-done` dates older done tasks. Relay test scripts live in vaultdbai/nandi-relay (`scripts/test-relay-*`). `scripts/gen-bison-css.py` generates mascot CSS.
- **Where:** `scripts/complete-task.js`, `scripts/gen-bison-css.py`; `docs/tasks.md`.
- **Preconditions:** None.
- **Steps:** `pnpm task:complete <existing-x-task>` in a scratch branch.
- **Expected:** File moved and renamed with today's date.
- **Edge / negative cases:** Ambiguous or missing name prints an error.
- **Automation:** Covered by: none | Layer: manual | Gap: none.
- **Needs relay:** no
- **Status:** Shipped

### QA-09 — Offline suite (`pnpm run test:offline`)

- **What:** Proves Nandi works with no relay. It boots the core in-process with a fresh home and drives the HTTP API: with no relay, and as a build whose default relay and join code point at a port nothing listens on. Every screen's read endpoints answer within 5s; an agent runs on the canned engine, asks for approval, is approved and completes; a two-step workflow runs; a starter pack installs from a local folder (`test-fixtures/offline-pack`); person-to-person endpoints answer `RelayUnavailable` (RL-36). No model, no relay, no internet.
- **Where:** `src-tauri/src/offline_suite.rs`; `package.json` (`test:offline` = `cargo test --lib offline_suite::`); `.github/workflows/ci.yml` (Offline suite step). Also runs inside `pnpm run rust:test`.
- **Preconditions:** `NANDI_RELAY_URL` unset (the suite refuses to run with it set).
- **Steps:** `pnpm run test:offline`.
- **Expected:** All tests pass in well under a minute.
- **Edge / negative cases:** Routes that read another service on purpose (the GitHub starter-pack registry, AWS/Bedrock, coding-agent CLIs) are left out; none is the relay.
- **Automation:** Covered by: itself | Layer: api | Gap: the desktop window isn't driven (ui-e2e).
- **Needs relay:** no
- **Status:** Shipped

## Gaps and planned (not testable yet)

- **Updater endpoint (fixed 2026-11-03).** `tauri.conf.json` now checks the GitHub release's `latest.json`, which `release.yml` attaches to every release. Publishing the draft is what makes an update visible.
- **"Sync now" no longer exists.** Comments in `services/repos/git.rs` / `repo_service.rs`, and userguide §12 ("Sync now fails with a conflict") describe it; the code has no such command. Backup is Settings › Backup (Back up now / Restore from remote).
- **No UI for rebuild, git diff or git commit.** `repo_rebuild`, `repo_git_diff`, `repo_git_commit` exist as commands/routes only.
- **Validation failures are silent in the UI.** `repo://validation_failed` is only logged by `src/lib/agent-sync.ts`; users see a count in This computer’s copy but no toast or per-file detail.
- **Template metadata in the bundle:** `tauri.conf.json` `publisher: "Your Name"`, template short/long descriptions and copyright; `package.json` author "Danny Smith"; download names `VaultDB-Nandi-*`.
- **Assistant alias** (`docs/tasks-todo/task-x-nandi-alias.md`): only the mascot name is configurable today; the Ask prompt, companion, notifications and backend fallback names still hard-code "Nandi".
- **Keyring not isolated per `NANDI_HOME`.** Two homes on one machine share LLM keys and the backup credential (same keychain service, `com.nandi.app`); no task tracks this, but test plans that assume isolated homes should account for it.
- **Schema refusal on the desktop app has no dialog.** A `state.db` from another schema is refused at startup (DT-12), but in the desktop app the error ends startup with a panic ("error while building tauri application"); a Windows release build has no console, so the user just sees the app not open. The message reaches the user only from a terminal or `--headless`.
