# Domain Model

The gold copy's types live in `src-tauri/src/services/domain/`: a pure layer with no I/O. It parses, validates, and renders file contents and enforces invariants; it never touches the filesystem, SQLite, git, the network, or Tauri. `domain::tests::domain_has_no_io_deps` fails the build if it does.

Allowed dependencies: std, serde (+ `serde_yaml`, `serde_json`), and pure-computation crates (`croner` for cron schedules, `glob` for patterns, `regex`).

See [data-model.md](./data-model.md) for where files live and how SQLite follows them.

## Layers

```
commands / http  ──▶  services (agents.rs, workflows.rs, triggers/, preferences.rs, …)
                          │  domain validates → repository saves → RepoStore::commit
                          ▼
                 RepoStore (repo_store.rs) ── AgentRepo aggregate in memory
                   │            │                      │
     repos/ (files + git)   projection.rs (SQLite)   EventPublisher (bus, reactor)
                   │
                domain/ (pure types)
```

| Layer           | Owns                                                        | Never                           |
| --------------- | ----------------------------------------------------------- | ------------------------------- |
| `domain/`       | Types, parsing, rendering, invariants                       | I/O of any kind                 |
| `repos/`        | Reading/writing gold-copy files, atomic writes, git staging | SQLite, events                  |
| `projection.rs` | Projected SQLite tables, `sync_state`                       | Files, runtime tables           |
| `repo_store.rs` | Aggregate in memory, reload/commit sequencing, publishing   | Writing files (repositories do) |
| `watcher.rs`    | Mapping file events to reloads                              | Writing anything                |
| `migration.rs`  | One-time layout upgrade                                     | —                               |

## Value Objects

Immutable and validated at construction: holding one means it's valid.

| Type                                                           | Rule                                                                                     |
| -------------------------------------------------------------- | ---------------------------------------------------------------------------------------- |
| `AgentId`                                                      | lowercase letters, digits, hyphens; 1–64; starts with a letter or digit                  |
| `WorkflowId`, `StepId`, `TriggerId`, `ConnectorId`, `PilotRef` | letters, digits, `-` `_` `.`; 1–128; no leading dot                                      |
| `ModelRef { provider, name }`                                  | `deepseek:` / `claude:` / `gpt:` / `ollama:` + a name; also `{provider, name}` in YAML   |
| `ContainerSpec { image, memory_mb, cpu }`                      | image without spaces; 16 ≤ memory_mb ≤ 1 048 576; 0 < cpu ≤ 256                          |
| `CapabilitySet { tools, pilots, network, aws }`                | hosts normalized (`*.example.com`, `:port`), AWS services non-empty, no duplicate pilots |
| `CronSpec`                                                     | 5 fields, or 6 with seconds first                                                        |
| `SecretRef`                                                    | `nandi.` + lowercase letters, digits, `.` `_` `-`: a keyring entry name, never a secret  |

IDs become folder and file names, so validation is also path-traversal protection. Deserializing goes through the same rule (`#[serde(try_from = "String")]`).

## Entities

| Entity                                        | File(s)                                            | Behavior                                                                                                                                                                                                                                                    |
| --------------------------------------------- | -------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Agent`                                       | `skill.md` (frontmatter + body), `instructions.md` | `new`, `from_files`, `to_skill_md`, `to_instructions_md`, `update_instructions`, `update_profile`, `set_model`, `requires_approval_for`, `allows_tool`, `system_prompt`, `effective_model`                                                                  |
| `Playbook`                                    | `skills/<name>.md` next to `skill.md`              | `new`, `from_md`, `to_md`, `index_line`; `check_unique`, `prompt_index`: name = file stem (kebab-case), `description` and `when` one line, body ≤ 900 words, no `model:`                                                                                    |
| `Workflow`                                    | `workflows/<id>.yaml`                              | `from_yaml`, `to_yaml`, `validate_step_graph` (duplicate steps, `{{step.output}}` pointing at itself or a later step, missing agents), step `on_failure: fail \| continue`                                                                                  |
| `Trigger`                                     | `triggers/<id>.yaml`                               | `from_yaml`, `to_yaml`, `validate`, `secret_ref`; typed `TriggerSource` (`Cron`, `File`, `Webhook`, `Device`, `Event`)                                                                                                                                      |
| `Settings`                                    | `settings.yaml`                                    | portable settings plus `secret_refs`; refs are never resolved here                                                                                                                                                                                          |
| `ConnectorManifest`                           | `connectors.yaml`                                  | declarations; read by `ConnectorRegistry` (`services/connectors.rs`) and, for a stdio connector's `config.network`, the egress proxy                                                                                                                        |
| `Policy`, `PolicySet`                         | `policies/<id>.md`                                 | `Policy::parse`, `is_enforced`, `applies`; `PolicySet::parse` (broken files kept apart, they block), `effective` (what an agent ends up with, and which policy decided each change)                                                                         |
| `GroupCopy`, `GroupAccess`, `GroupPolicyText` | `groups/<group-id>.md`, `groups/access.yaml`       | `GroupCopy::parse`/`render` (policy text kept byte for byte), `GroupAccess::level` (default Ask first), `GroupPolicyText::parse` (`requests:` block), `RequestRules::decide`, `AccessLevel::decide`, `Decision::stricter`, `verify` ([groups](./groups.md)) |

Fields are private; change them through methods that keep invariants. Unknown `skill.md` frontmatter keys are kept and written back, so saving never drops what the user (or a newer version) put there.

```rust
// ❌ BAD: building an entity from loose strings and hoping
let agent = Agent { id: input.id.clone(), name: input.name.clone(), .. };

// ✅ GOOD: value objects and the constructor enforce the rules
let mut draft = AgentDraft::named(AgentId::parse(&input.id)?, &input.name);
draft.model = input.model.as_deref().map(ModelRef::parse).transpose()?;
let agent = Agent::new(draft)?;
```

## Aggregate: `AgentRepo`

The whole gold copy in memory (`domain/repo.rs`). It owns cross-entity invariants:

| Invariant                                                   | Commands (`check_references`, `check_removable`)             | Loaded files                             |
| ----------------------------------------------------------- | ------------------------------------------------------------ | ---------------------------------------- |
| Unique IDs per kind (workflows across global + agent scope) | rejected                                                     | first wins; the rest become `Problem`s   |
| Workflow steps and triggers name existing agents/workflows  | allowed (new workflows log a warning); checked when they run | `Severity::Warning` problem; entity kept |
| Deleting an agent a workflow or trigger uses                | refused with `DomainError::InUse`                            | n/a                                      |

Why lenient for files: a `git pull` can deliver a workflow before its agent. Rejecting it would make entities flicker in and out depending on file order.

The aggregate is pure: `RepoStore` loads it (`Repos::load_aggregate`), reloads single entities (`reload(kind, id, changed_by)`), and projects it. The spec's `AgentRepo::load` / `rebuild_projection` / `reload_entity` therefore live on `RepoStore` (`load`, `rebuild`, `reload`).

## Repositories

`repos/` has one repository per entity, all over a `RepoLayout` (gold: `~/.nandi/repo/.nandi/…`; legacy: the pre-v6 folders, used only if migration hasn't finished).

| Trait                 | Methods                                          | Implementations                                              |
| --------------------- | ------------------------------------------------ | ------------------------------------------------------------ |
| `Repository`          | `load`, `load_all`, `save`, `delete`, `exists`   | `AgentRepository`, `WorkflowRepository`, `TriggerRepository` |
| `SingletonRepository` | `load` (missing file = defaults), `save`, `path` | `SettingsRepository`, `ConnectorRepository`                  |

Every write: temp file + fsync + rename (`fs_atomic::write_if_changed`, which skips identical content so the watcher and git see no churn), then `git add` (`git::GitIndex`). `AgentRepository::delete` moves the folder to the trash and stages the removal. `WorkflowRepository::create_source` writes user-authored YAML verbatim (comments kept) once it parses.

## Ports

| Trait            | Implemented by                            | Used by                      |
| ---------------- | ----------------------------------------- | ---------------------------- |
| `Repository`     | the repositories above                    | services, `RepoStore`        |
| `ProjectionSink` | `SqliteProjection`                        | `RepoStore`                  |
| `EventPublisher` | `EventBus`, `Reactor` (`services/mod.rs`) | `RepoStore` publishes to all |

The reactor keeps running parts in step with any change, from a command or a file: new agents get an orchestrator, changed triggers re-arm their sources.

## Errors and Events

`DomainError` is typed (`InvalidId`, `Invalid`, `Parse`, `Duplicate`, `DanglingReference`, `Cycle`, `NotFound`, `InUse`, `Rule`); `RepoError` adds I/O and file-level cases. Both convert to `String` only at the transport edge.

| Topic                                                       | When                                                                       |
| ----------------------------------------------------------- | -------------------------------------------------------------------------- |
| `repo://{agent,workflow,trigger}.{created,updated,deleted}` | an entity changed (payload: `kind, id, action, sourcePath, changedBy`)     |
| `repo://settings-updated`, `repo://connector-updated`       | a singleton changed                                                        |
| `repo://validation_failed`                                  | a file failed to load; the last good version stays                         |
| `repo://file_changed`                                       | the watcher reloaded an entity (`result: changed \| unchanged \| invalid`) |
| `repo://rebuild_started`, `repo://rebuild_completed`        | a rebuild ran                                                              |

`changedBy` is `user` (a command), `watcher` (an editor or script), or `git` (pull / Sync now). The bus also keeps emitting the list-reload events the UI already uses (`agent://agents_changed`, `workflow://changed`, `trigger://changed`).

## Adding a Field to an Entity

1. Add it to the domain type with validation in the constructor, and to the file struct (`Frontmatter`/`FrontmatterOut`, `WorkflowFile`, `TriggerFile`, `SettingsFile`) with `skip_serializing_if` for defaults so existing files don't churn.
2. Add a round-trip assertion to the entity's tests.
3. If it should be queryable, add a projection column in `db.rs` (with a `migrate_v*` `ALTER`), write it in `projection::upsert_row`, and bump `PROJECTION_VERSION`.
