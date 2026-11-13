# Live UI

Open screens stay current when files change on disk, whoever changed them: you, an agent, a workflow, or someone outside Nandi (an editor, a script, a `git pull`). Every change is one typed event, `live://changed`, and the frontend refetches exactly the queries it affects. A **Refresh** button and F5 / Ctrl+R re-read everything from disk, for when a watcher misses something.

Code: `src-tauri/src/services/live/` (Rust), `src/lib/live-changes.ts`, `src/hooks/useLiveChanges.ts`, `src/components/live/` (frontend).

## What is watched

| Data                                                                                                             | Watched by                                            | Reaches the feed through                              |
| ---------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------- | ----------------------------------------------------- |
| Agents (`skill.md`, `instructions.md`), workflows, triggers, `settings.yaml`, `connectors.yaml`, `manifest.yaml` | the repo watcher (`services/watcher.rs`)              | the store (`live::StoreChanges`, an `EventPublisher`) |
| Policies (`.nandi/policies/`)                                                                                    | the repo watcher                                      | `ChangeFeed::file_changed` from the watcher           |
| `installed.yaml` (installed packs), `NANDI.md` (your profile)                                                    | the repo watcher (`Target::Packs`, `Target::Profile`) | `ChangeFeed::file_changed` from the watcher           |
| Capabilities                                                                                                     | the repo watcher                                      | their own `capability://*` events (not in the feed)   |
| My notes (`~/.nandi/notes`, or the folder shared as `my-notes`)                                                  | the folder watcher (`live/folders.rs`)                | `live::publish_folder_changes`                        |
| Agents' work folders (`~/.nandi/local/work/<agent>/`)                                                            | the folder watcher                                    | `live::publish_folder_changes`                        |

Runs, history, the Inbox and the dashboard live in SQLite. Their own events (`agent://*`, `dashboard://*`, `workflow://*`, `gateway://*`) already keep them current.

## Rules for watchers

- **Only Nandi's folders.** The repo, `~/.nandi/local/work` and the notes folder. `folders::watchable` refuses a drive root or the home folder, even when someone shared one as My notes. Refresh covers what isn't watched.
- **Batch, then act.** `watcher::next_batch` waits until the files are quiet for 500 ms (at most 2 s). Both watchers use it.
- **Ignore noise.** `watcher::ignored`: `.git/`, editor temp files (`.*.swp`, `*.tmp`, `*~`, `.#*`), the app's `.nandi-tmp-` files, `runs/`. Work folders also skip `node_modules`, `target`, `__pycache__`, `.venv`, `venv`, `.cache` and `dist`. Notes skip hidden files.
- **Re-scan when events may be lost.** A notify rescan flag, a watcher error, or more than `BURST_LIMIT` (512) paths in one batch (a Windows change buffer overflows without saying so) re-scans the whole area. The log says why: `Re-scanning <folder>: the OS dropped file events`.

## Who changed it

`ChangeAuthor` is `you`, `agent`, `workflow` or `outside`.

| Change                               | Author                                                                                  |
| ------------------------------------ | --------------------------------------------------------------------------------------- |
| A command wrote a definition         | `you` (the store's `ChangedBy::User`)                                                   |
| The watcher reloaded a definition    | `outside` (`ChangedBy::Watcher` or `Git`)                                               |
| The app wrote any other watched file | `you`: `fs_atomic` and `NotesFolder` note each write in `live::own_writes` (5 s window) |
| A file in `work/<agent>/`            | that agent's run going at the time, else `outside` (`live::author`)                     |
| A note, agents may write             | Chief of Staff's run going at the time, else the latest run, else `outside`             |
| A note, read-only share              | `outside`                                                                               |

A run started by a workflow shows as the workflow. "Going at the time" is running, waiting for approval, or ended in the last 10 s (`author::RECENT_RUN_MS`). Agents write from the container or through the `filesystem` connector, and neither says which run wrote what, so this is the best the app knows.

## The event and the queries

`live://changed` carries a `LiveChange`: `area`, `targetId` (entity ID, note name, or the agent for a work folder), `label`, up to five `paths` and `pathCount`, `action`, `author`, `at`.

Event names use only letters, digits, `-`, `/`, `:` and `_`. Tauri refuses any other name and the event is lost, so the store's topics are `repo://agent-created`, never `repo://agent.created`.

```rust
// ✅ GOOD
"repo://connector-updated"
// ❌ BAD: Tauri drops it ("only alphanumeric, '-', '/', ':', '_' permitted")
"repo://connector.updated"
```

`queryKeysFor(change)` in `src/lib/live-changes.ts` is the one map from a change to its queries. Add a case there when a new screen shows file-backed data.

```typescript
// ✅ GOOD: exactly the note and the list
case 'notes':
  return id ? [notesKeys.list, notesKeys.note(id)] : [notesKeys.all]

// ❌ BAD: polling, or refetching everything on every change
refetchInterval: 15_000
```

- Lists still follow their own events (`agent://agents_changed`, `workflow://changed`, `trigger://changed`). `queryKeysFor` covers the detail queries.
- `repo://file_changed` keeps refreshing git status and the stored entity, including for invalid files (which don't reach the feed).
- `applyLiveChange` puts the change at the top of `['liveChanges', 'recent']` (only once that list has loaded).

## Recent changes and "Updated by"

- The core keeps the last 200 changes in memory (`ChangeFeed`, `list_recent_changes`, `POST /api/rpc/list_recent_changes`). The list starts empty with the app.
- **Home** shows the latest 8 in **Recent changes** (`RecentChanges.tsx`), each linking to its page (`live-model.ts` `viewOf`). Hidden until something changes.
- **`UpdatedBy`** shows "Updated just now by Chief of Staff" on a note (My notes) and on an agent's page (its definition or work folder). Without a change this session, it falls back to the file's own time.

## Refresh

`refresh_from_disk` (`POST /api/rpc/refresh_from_disk`) re-scans every definition (`RepoStore::rescan_all`), whether a watcher runs or not. Then `refreshFromDisk()` refetches every query in `FILE_BACKED_KEYS`. Notes and work folders are read from disk on each request, so refetching them is enough.

| Where                                                       | How                                                                    |
| ----------------------------------------------------------- | ---------------------------------------------------------------------- |
| My notes, an agent's page, Workflows, Triggers, Home's list | `RefreshButton`                                                        |
| Anywhere in the main window                                 | F5, Ctrl/Cmd+R (`refresh-from-disk`), instead of reloading the webview |
| The command palette                                         | **Refresh**                                                            |

## Tests

| What                             | Where                                                                                                                                                      |
| -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Folder watcher with real folders | `live/folders.rs` `changes_in_real_folders_arrive_batched_by_area`                                                                                         |
| Who changed it                   | `live/tests.rs` `notes_say_who_changed_them`, `files_in_a_work_folder_name_the_agent_or_workflow_running`; `live/author.rs`                                |
| Refresh with the watcher off     | `live/tests.rs` `refresh_works_with_the_watcher_off`                                                                                                       |
| Dropped events                   | `watcher.rs` `events_the_os_may_have_dropped_rescan_everything`; `live/folders.rs` `a_batch_that_lost_events_rescans_every_area`                           |
| Event names Tauri can emit       | `desktop/events.rs` `every_event_name_is_one_tauri_accepts`, `domain/events.rs` `every_topic_is_a_valid_tauri_event_name`; `src/lib/client/events.test.ts` |
| Exact invalidation               | `src/lib/live-changes.test.ts`                                                                                                                             |

File watcher tests wait with `live::testing::wait_until` (polls a condition, up to 20 s), never a fixed sleep.
