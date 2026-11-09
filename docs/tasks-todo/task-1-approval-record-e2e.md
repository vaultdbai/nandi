# Task: One end-to-end test of approvals and the decision record

**From:** feature review against "trust, state, verification" (2026-11-08)
**Serves:** trust, state, verification
**Size:** S–M

Approvals are the main trust control, and the path from asking to the written record has no single test.

## What to do

One API-level test (in the offline suite's style, no relay, canned engine) that:

- [ ] Starts a run whose tool call needs approval (**CH-08**), and one through a connector (**CP-06**).
- [ ] Approves one and refuses the other; the run goes on or stops as it should.
- [ ] Reads back the written `decisions.log` line for each (**PO-14**): who, what, the decision.
- [ ] Applies an auto-approve rule and checks it is recorded as such (**RQ-32**).
- [ ] Crashes and restarts a run until the restart cap stops it (**CH-10**).

Then update the "Automation" lines in `docs/testing/`.

## Done when

A broken approval or a missing decision line fails CI.
