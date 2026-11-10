# Task: One end-to-end test of approvals and the decision record

**From:** feature review against "trust, state, verification" (2026-11-08)
**Serves:** trust, state, verification
**Size:** S–M

Approvals are the main trust control, and the path from asking to the written record has no single test.

## What to do

One API-level test (in the offline suite's style, no relay, canned engine) that:

- [x] Starts a run whose tool call needs approval (**CH-08**), and one through a connector (**CP-06**).
- [x] Approves one and refuses the other; the run goes on or stops as it should.
- [x] Reads back the written decision for each (**PO-14**): who, what, the decision. (`decisions.log` was folded into the monthly gold-copy files, `.nandi/decisions/<yyyy-mm>.md`, the one record of decisions; the test reads those.)
- [x] Applies an auto-approve rule and checks it is recorded as such (**RQ-32**).
- [x] Crashes and restarts a run until the restart cap stops it (**CH-10**).

Then update the "Automation" lines in `docs/testing/`.

**Done 2026-11-09:** `src-tauri/src/approvals_suite.rs` (runs in `pnpm run rust:test`), plus an Inbox Ask-chat decision and group-forced and group-auto approvals.

## Done when

A broken approval or a missing decision line fails CI.
