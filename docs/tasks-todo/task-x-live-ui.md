# Task: The screen always shows the latest, whoever changed it

**From:** owner, 2026-11-04: "When files change on disk, such as someone editing or adding new files, the UI doesn't refresh and show new things. Maybe there should be a refresh button somewhere, or a file watcher that keeps the UI up to date when files, agents, notes and so on change, with events so the user knows the latest done by many agents, workflows or manual edits."
**Size:** M.
**Repo:** `nandi` (app). Move this task into `nandi/docs/tasks-todo/` when work starts.
**Builds on:** the repo watcher (`src-tauri/src/services/watcher.rs`: debounced events for agents, workflows, triggers, capabilities and policies under `.nandi/`, published as `repo://*` events), the event bus (`services/events.rs`) and the frontend event bridge (`src/lib/client/events.ts`, `src/lib/agent-sync.ts`).

## What to find out first

- [ ] List every place the app shows data that lives in files or changes in the background, and for each one: is it watched, is an event published, does the screen listen? At least: agents and their skill.md, workflows, triggers, capabilities, policies, **My notes** (the Chief of Staff's notes folder), agents' work folders and run files, installed packs, connectors.yaml, settings.yaml, NANDI.md, the Library, the Inbox, runs and history.
- [ ] Reproduce the owner's case: edit and add files from outside the app (an editor, git pull, another agent or workflow), and note which screens stay stale.

## Changes

- [ ] Watch what isn't watched yet, starting with My notes and agents' work folders (debounced; ignore editor temp files; never watch outside the Nandi folders).
- [ ] Every watched change publishes a typed event; the frontend invalidates exactly the queries that event affects (TanStack Query keys), so open screens update by themselves without a reload.
- [ ] A small, calm "Updated just now by …" line where it helps (notes, agent pages): "by you", "by Chief of Staff", "by the Weekly report workflow", or "outside Nandi" for manual edits, so the person knows who changed what.
- [ ] An activity list (on Home or the companion) of recent changes by agents, workflows and manual edits, linking to each.
- [ ] A **Refresh** button (and F5 / Ctrl+R in the app window) on screens backed by files, as a fallback when the watcher misses something (network drives, sleeping laptops). It re-reads from disk, not just from the cache.
- [ ] Handle watcher limits: if the operating system drops events (too many changes at once, a folder replaced by a git checkout), re-scan that area and say so in the log.

## Acceptance

- [ ] Editing a note, an agent's skill.md or a workflow in an outside editor updates the open screen within about a second, with no reload.
- [ ] Files added by an agent or workflow appear without a reload, with who made them.
- [ ] The Refresh button brings every screen up to date even with the watcher switched off (a test).
- [ ] Testing catalog entries added ("Needs relay: no"), `pnpm run check:all` and `pnpm run test:offline` pass.
