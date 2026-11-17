# Task: Nandi's own work heals itself

**From:** owner, 2026-11-17. "If some run or task breaks in between then it never gets retried or cleaned up, for example my morning digest showing up red in to-do instead of retrying, as it is Nandi's work." Also: no way to resync email by hand.
**Serves:** state (a failure is retried and closed, the record says which attempt is current) and trust (the user's limits and own chats are never retried behind their back).

## What was wrong

| Problem                                                       | Cause                                                                          |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------ |
| A failed scheduled run stayed red                             | Nothing retried runs; `fail_interrupted_runs` failed them at start and stopped |
| Retry on a failed trigger run in the Inbox did nothing useful | It called `retry_task`, which only knows delegations                           |
| The morning digest never came if the app was closed at 8:00   | The cron job started from "now"; a missed time was lost                        |
| A laptop asleep at 8:00 fired late                            | One long `tokio::time::sleep` stops while the machine sleeps                   |
| A failed email run lost its mail                              | The cursor moves once the run is queued, not when it succeeds                  |
| No manual resync of mail                                      | Only Check now (new mail since the cursor)                                     |

## What was built

- `services/run_retry/`: failed runs from Nandi's own sources are tried again after 2 and 10 minutes (3 attempts), within 6 hours, in the same conversation, with the first input and a note on what failed. The model decides what to do differently. Policy stops (caps, read-only) and the user's own chats are never retried on their own. The database is the schedule (`runs.retry_of`, `runs.attempt`, migration 007), so retries survive a restart.
- `run_waiter::newest_run` follows retries, so Work, the Inbox and the overlap check see the latest attempt. A failure waiting on a retry is `waiting` (`auto`), not red.
- `retry_run`: the Inbox's Retry for trigger runs and failed email or calendar checks.
- Cron catch-up at start (`cron::missed`) and wall-clock waits.
- Email **Read again** (`read_email_again`): the last 3 days again, cursors untouched, the model told it may already have them.
- Sidebar (owner, same day): Home first, then Nandi with My notes under it.

See [triggers.md](../developer/triggers.md#when-a-run-fails-or-a-time-is-missed) and TR-29, TR-30, CP-37 in `docs/testing/`.

## Decisions taken (owner away; review)

- **3 attempts, 2 and 10 minutes apart, 6-hour window.** Long enough for a model outage or a restart, short enough that a digest isn't stale.
- **Retry everything Nandi started on its own, not only schedules.** File, webhook, device and event triggers and the email and calendar checks too. A retry may repeat a step that wrote something: tools that write still ask for approval, and the note tells the model to check what was done first.
- **No error classifier.** Whether a failure is worth retrying is not parsed from the error text (workspace rule: no heuristics for what a model reads). Only facts decide: the source, a policy stop, the window and the count.
- **Catch-up only within 12 hours and once per start.** Opening the laptop at 21:00 doesn't send a morning digest.
- **Read again is 3 days, not 14.** Enough to repair a bad day without a large model bill.

## Follow-ups (not done)

- [ ] Work doesn't refetch on `agent://turn_done`, so a trigger item's state shows on the next refetch (existing behaviour, now more visible with retries).
- [ ] A failed workflow step's agent run is not retried (workflows resume, but a failed step stays failed).
- [ ] Settings for the retry policy, if anyone asks. Not built: no one asked.
