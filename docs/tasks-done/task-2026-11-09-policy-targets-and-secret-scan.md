# Task: Policies never fail silently; no secrets in the repo

**From:** feature review against "trust, state, verification" (2026-11-08)
**Serves:** trust, verification
**Size:** S

## What to do

- [x] **PO-04:** a policy with a `workflow:` target matches nothing today (runs don't carry their workflow) and is silently ignored. Until runs carry it, refuse the policy on save with a clear error, and say so in Settings. A rule the user thinks applies but doesn't is a trust bug.
- [x] **DT-04:** a test that saves every kind of definition (agents, workflows, triggers, connectors, settings) with secrets set, then scans `~/.nandi/repo/` and the git history for the secret values. Files hold `secret_refs` only.
- [x] Update PO-04 and DT-04 in `docs/testing/`.

## Done when

A `workflow:` target can't be saved, and a secret written to the repo fails CI.

**Done 2026-11-09:** `Target::parse` refuses `workflow:` (Studio's policy form says so under Applies to; policies are edited in Studio, there is no Settings section for them); `src-tauri/src/secrets_suite.rs` scans the repo and its history (runs in `pnpm run rust:test`).
