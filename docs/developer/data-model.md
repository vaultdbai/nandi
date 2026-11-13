# Data Model: Gold Copy and Projection

Files in the user's git repo are the **single source of truth** for everything definitional. SQLite holds a **projection** of those files (a materialized, queryable view) plus runtime data that only exists locally. Delete `state.db` and Nandi rebuilds the projection from the files; nothing definitional is lost.

```
~/.nandi/repo/  (gold copy, git)     ──read──▶  AgentRepo (memory)  ──project──▶  SQLite (projection)
       ▲                                           ▲
       └── repositories write ◀── commands         └── file watcher reloads on edit / git pull
```

See [domain-model.md](./domain-model.md) for the types.

On every start, `services::migration::bootstrap` opens `~/.nandi/repo`, or creates it as a git repo on `main`, writes whatever it lacks of `.nandi/settings.yaml` (keyring references only), `.gitignore` and `.nandi/manifest.yaml`, and gives a new repo the commit "Initialize Nandi repo".

## What Lives Where

| Data                                                                          | Where                                                 | In git | Rebuildable               |
| ----------------------------------------------------------------------------- | ----------------------------------------------------- | ------ | ------------------------- |
| Agents (`skill.md`, `instructions.md`)                                        | `~/.nandi/repo/.nandi/agents/<id>/`                   | ✅     | source                    |
| Agent playbooks                                                               | `~/.nandi/repo/.nandi/agents/<id>/skills/<name>.md`   | ✅     | source                    |
| Agent-scoped workflows                                                        | `~/.nandi/repo/.nandi/agents/<id>/workflows/<w>.yaml` | ✅     | source                    |
| Global workflows                                                              | `~/.nandi/repo/.nandi/workflows/<w>.yaml`             | ✅     | source                    |
| Triggers                                                                      | `~/.nandi/repo/.nandi/triggers/<t>.yaml`              | ✅     | source                    |
| Portable settings (theme, language, default model, provider pins)             | `~/.nandi/repo/.nandi/settings.yaml`                  | ✅     | source                    |
| Connector declarations                                                        | `~/.nandi/repo/.nandi/connectors.yaml`                | ✅     | source                    |
| Groups I'm in: each group's signed policy and members ([groups](./groups.md)) | `~/.nandi/repo/.nandi/groups/<group-id>.md`           | ✅     | refreshed from the relay  |
| My access level per group                                                     | `~/.nandi/repo/.nandi/groups/access.yaml`             | ✅     | source                    |
| Every Inbox decision: who, what, why, and the chat it came from               | `~/.nandi/repo/.nandi/decisions/<yyyy-mm>.md`         | ✅     | source (append-only)      |
| Repo format version, owner                                                    | `~/.nandi/repo/.nandi/manifest.yaml`                  | ✅     | source                    |
| Marketplace installs (read-only until forked)                                 | `~/.nandi/repo/.nandi/agents/marketplace/…`           | ✅     | source                    |
| What was installed from where                                                 | `~/.nandi/repo/.nandi/installed.yaml`                 | ✅     | source                    |
| Registry index cache, fetch scratch repos                                     | `~/.nandi/local/cache/`                               | ❌     | ✅ re-fetched             |
| Projection (`agents`, `workflows`, `triggers`, …)                             | `~/.nandi/local/state.db`                             | ❌     | ✅ `nandi rebuild`        |
| Group requests that went ahead, per member and day                            | `state.db` → `group_requests`                         | ❌     | ❌ runtime history        |
| Runs, messages, approvals, queue, fires, workflow runs                        | `~/.nandi/local/state.db`                             | ❌     | ❌ runtime history        |
| Inbox Ask chats (`inbox:<kind>:<id>`, `intake:<id>`): their messages          | `state.db` → `conversations`, `messages`              | ❌     | ❌ stays on this computer |
| Engine choice, canned script                                                  | `state.db` → `local_settings`                         | ❌     | ❌ machine-local          |
| First-run step reached (`onboarding.step`), per Nandi home                    | `state.db` → `local_settings`                         | ❌     | ❌ machine-local          |
| Dismissed hints (Get started, backup nudge)                                   | webview `localStorage` (`lib/dismissals.ts`)          | ❌     | ❌ hint comes back        |
| Audit logs (llm, egress, triggers)                                            | `~/.nandi/local/logs/`                                | ❌     | ❌                        |
| Agent workdirs                                                                | `~/.nandi/local/work/<id>/`                           | ❌     | ephemeral                 |
| Secrets (LLM/AWS keys, trigger secrets, remote credential)                    | OS keyring                                            | ❌     | ❌                        |
| Deleted agents, replaced repos                                                | `~/.nandi/trash/`                                     | ❌     | —                         |

The audit logs in `~/.nandi/local/logs/` (`llm`, `egress`, `triggers`, `pilots`, `connectors`) are plain JSONL written only through `audit::append`. Each line carries `prev_sha256`, the SHA-256 of the line before it (zeros for the first), and each append is fsynced. `audit::chain::verify` finds the first line that doesn't follow the one before it (an edited, removed or moved line). A failed write is counted against its log; the Logs view shows "Audit degraded" for lost entries or a broken chain (`get_audit_health`).

The repo's `.gitignore` (written by the bootstrap) keeps `.nandi/local/`, `*.db*`, `*.log`, `work/`, and Nandi's atomic-write temp files out of commits.

## Decisions

Every decision is appended to `.nandi/decisions/<yyyy-mm>.md`, the one record of them: a person's, on an Inbox item from its button or its Ask chat ([interaction-layer](./interaction-layer.md#inbox-ask-chats)), on an access request or a connector grant; an auto-approve rule's or the groups' (work they let through); and every call a policy refused. The owner decided this on 2026-11-08 (the record of who decided what, and why, belongs to the company's gold copy; the chats stay local) and on 2026-11-09 folded the old `decisions.log` into it. The file is written only by `repos::decision_repo` (atomic, staged like every gold-copy file, never projected), from `services::inbox::audit::record` (a person; **Decided by** is the local user's name) or `record_in` (a rule or policy, named in `Decided::by`: `auto-approve rule (mail from chase.com)`, `group policies`, `policy no-shell`), which each decision service calls once after the decision took effect (`chat::approve_action`, `workflows::approve_workflow_step`, `gateway::decide_gateway_hold`, `close_gateway_assignment`, `manual_steps::complete_manual_step`, `clarification::respond_to_clarification`, `trust_requests::respond_trust_request`, `returns::dismiss_request_return`, `requests::retry_task`, `work::dismiss_work`, `intake::approve_as` and `dismiss`, `intake::gate_task` (a rule's or the groups' approval), `reassign::reassign_task`, `policies::record_refused` (a tool, pilot or browser control a policy refused; kind `policy`), `access::decide` (a connector a policy forbids), `grants::record` (kind `connectorGrant`, `granted` or `removed`)). A chat decision runs inside `audit::in_chat`, which adds the conversation ID and the confirmed note. The entry format is `domain::decision` (pure):

```markdown
# Decisions, 2026-11

…

## 2026-11-08 14:32:05 UTC · gatewayHold

- **Decided by:** Dev Chechi
- **Item:** gatewayHold `task-123` (Sam Lee, ask-me: Can you send the Q3 numbers?)
- **Decision:** declined
- **Reason:** Not before the board meeting
- **Chat:** `inbox:gatewayHold:task-123`
```

A note's later lines are indented under its field, so it can't start a heading or a field of its own. Writing is best effort: the decision already happened, so a failure is logged. An approval that expired isn't a decision (nobody made one): it stays on the approval row and in the run's history. What stays out of the file on purpose: per-call usage logs (`connectors.log`, `egress.log`, `pilots.log`, `llm.log`), which say what was used, not who allowed it.

**Reading them back.** `domain::decision::parse_month` turns a month file into `DecisionRecord`s
(`at`, `kind`, `itemId`, `itemLabel`, `decision`, `decidedBy`, `reason?`, `chatId?`), the inverse of
`DecisionEntry::render`. It is tolerant of hand edits: text outside entries is ignored, an entry that has no
readable time, item or decision is skipped and logged (`repos::decision_repo::entries`), and the rest are returned.
`months()` lists the `<yyyy-mm>.md` files, newest first. The Decisions page (Home › Decisions) is the reader;
`DecisionFilter` (kind, decision word, text in the item or reason) is applied in `domain::decision::select`.

## Projected vs Runtime Tables

| Table                                                       | Kind       | Written by                  |
| ----------------------------------------------------------- | ---------- | --------------------------- |
| `agents`, `workflows`, `triggers`, `settings`, `connectors` | projection | `services::projection` only |
| `sync_state` (kind, id, source path, file hash)             | projection | `services::projection` only |
| everything else (`runs`, `trigger_state`, …)                | runtime    | the services that own them  |

Every projected row has `source_path` (the file it came from) and `projection_version`. Runtime tables never reference projected rows with cascading foreign keys: a rebuild must not delete run history (so `workflow_runs` has no cascading key to `workflows`).

```rust
// ❌ BAD: writing a definition straight into SQLite — the next rebuild erases it
conn.execute("UPDATE agents SET name = ?1 WHERE id = ?2", params![name, id])?;

// ✅ GOOD: domain validates → repository writes the file → commit projects and publishes
let mut agent = store.agent(id)?;
agent.update_profile(name, description)?;
store.repos().agents.save(&agent)?;
store.commit(EntityKind::Agent, id)?;
```

## Keeping the Projection in Step

| When                                                          | What happens                                                                                                                                                                                                                                                                                                                                                                                                                    |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| App start                                                     | `RepoStore::load` reads every file. Empty projection or a different `PROJECTION_VERSION` → full rebuild; otherwise only entities whose file hash differs from `sync_state` (drift while the app was closed) are re-projected, and rows whose file is gone are dropped.                                                                                                                                                          |
| A command changes something                                   | Repository writes the file atomically and stages it → `RepoStore::commit` reloads it, updates the aggregate and projection in one transaction, publishes `repo://<kind>-<action>`.                                                                                                                                                                                                                                              |
| A file changes outside Nandi                                  | The watcher (500 ms debounce) reloads it the same way, `changed_by: "watcher"`. An invalid file emits `repo://validation_failed` and the last good version stays live, in memory and in SQLite. Policies (`.nandi/policies/`) aren't projected: runs read them from disk, so the watcher only checks a changed one and emits `repo://file_changed` (`kind: policy`); a broken one blocks the agents it may cover (fail closed). |
| `git pull` / Restore from remote                              | Files change on disk; the watcher picks them up, and the sync re-scans everything with `changed_by: "git"`. Every change also reaches the screens as `live://changed`, with who made it (see [Live UI](./live-ui.md)).                                                                                                                                                                                                          |
| `nandi rebuild`, `POST /api/rpc/repo_rebuild`, `repo_rebuild` | Wipes the projection and rebuilds it. Refuses while a file is invalid unless `force` (which skips invalid files). Runtime tables are untouched.                                                                                                                                                                                                                                                                                 |

Bump `PROJECTION_VERSION` (`services/projection.rs`) whenever the shape of a projected row changes: every install then rebuilds on its next start.

## Secrets Boundary

`settings.yaml`, `connectors.yaml`, and trigger files hold **keyring references only** (`secret_refs: {deepseek_api_key: nandi.llm.deepseek}`, trigger `secretRef`, default `nandi.trigger.<id>`). The domain rejects a `secret` key in trigger configs and credential-looking connector config keys, and `settings.yaml` rejects unknown top-level keys, so a pasted API key fails validation instead of being committed.

Updating a secret in the UI writes the keyring only; the reference in the file stays the same, which is what makes the repo safe to push.

## Portable Paths

A path in the repo has to work on another machine, where Nandi may live in another folder. Shared folders (`connectors.yaml`) and a file trigger's watched folder are stored in the most portable form (`connectors::folders::portable`), with `/` separators:

| On this machine                   | Stored as           | Read back as (`connectors::expand_home`) |
| --------------------------------- | ------------------- | ---------------------------------------- |
| Inside the Nandi home             | `notes` (relative)  | `<Nandi home>/notes`                     |
| Inside the user's home            | `~/Documents/share` | `<user's home>/Documents/share`          |
| Anywhere else (`D:\work\reports`) | `D:/work/reports`   | the same: it only exists on this machine |

The Nandi home is `NANDI_HOME` when set, else `~/.nandi`. Machine-only state (the database, logs, work folders) stays in `.nandi/local/` and `~/.nandi/local/`, outside the repo.

## Backup

The repo is a workspace, not a mirror. Settings › Backup sets a git remote (`services::remote`): **Back up now** commits local changes and pushes them to `<prefix>/<slug>`, never to the default branch; **Restore from remote** pulls. A conflict aborts cleanly and lists the files.

Every app write is staged (`git add`), so `git status` in `~/.nandi/repo` shows exactly what the app changed.
