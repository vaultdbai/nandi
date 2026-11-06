# Triggers

A trigger is a rule that watches for something and dispatches work when it fires: an agent run (queued in that agent's [queue](./agent-runtime.md#one-turn-per-agent)) or a workflow run. The code is in `src-tauri/src/services/triggers/`. It has no Tauri imports.

```text
cron task ─┐
calendar  ─┤ (+ handover watches)
file watch ┤                              ┌─ storm guard / rate limit
webhook  ──┼─▶ TriggerEvent ─▶ dispatcher ─▶ trigger_fires row ─▶ chat::submit (agent queue)
device   ──┤    (mpsc, 1024)              │                     └ or WorkflowEngine::run_workflow
event bus ─┘                              └─ trigger://fired | rate_limited | error
```

## Sources

| Type       | Config (`sourceConfig`)                                                                                  | Fires when                                                                     | Default priority |
| ---------- | -------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------ | ---------------- |
| `cron`     | `{schedule}`                                                                                             | The schedule's next occurrence (local time)                                    | 80               |
| `calendar` | `{times, days?, skipDates?, people?, rotate?, startDate?, byDay?, cover?, backup?, backupAfterMinutes?}` | A listed time on a scheduled day, on this Nandi's duty days (see below)        | 80               |
| `file`     | `{path, patterns?, events?, debounceMs?, recursive?}`                                                    | A matching file is created/modified/deleted (debounced per path, default 2s)   | 70               |
| `webhook`  | `{}` (+ generated `secret`)                                                                              | `POST /api/triggers/webhook/{id}`                                              | 60               |
| `device`   | `{deviceId, events?}` (+ generated `secret`)                                                             | `POST /api/triggers/device/{deviceId}` with `{event, data}`, event in `events` | 50               |
| `event`    | `{eventName, filter?}`                                                                                   | `agent.turn_done`, `workflow.done`, or `workflow.failed` whose payload matches | 60               |

- **Cron** uses [`croner`](https://docs.rs/croner): standard POSIX fields, so `0 9 * * 1-5` is 9:00 on weekdays. An optional leading seconds field allows `* * * * * *`. The popular `cron` crate was avoided: it counts Sunday as 1, which silently shifts weekday schedules. Payload: `{scheduled_at, trigger_id}`.
- **Calendar** is a schedule plus who is responsible on each day (rules and examples in `domain::calendar`). For a given day, a stand-in (`cover`) comes first, then a fixed weekday owner (`byDay`), then whose turn it is (`people` from `startDate`, a `week` or a `day` each; daily turns skip days off). When the calendar names people but nobody is responsible that day, it doesn't fire. People are named the way the relay shows them: display name, handle or instance ID. This is a schedule, not a directory. Payload: `{scheduled_at, trigger_id, duty_id, date, role, responsible, backup, covering_for, duty_reason}`, where `duty_id` is `<trigger>/<YYYY-MM-DD>/<HH:MM>`. See [Calendar backups](#calendar-backups).
- **File** uses `notify` on the configured folder only. The folder must exist when the trigger is saved; if it disappears later, the watcher logs a warning and the rule stays. Patterns match the file name (case-insensitive on Windows). Payload: `{path, event, size, modified_at}`.
- **Webhook/device** receivers skip the web-token check (`http::auth::is_trigger_receiver`) and require the trigger's secret in `X-Trigger-Secret` instead (constant-time compare). They answer `202` as soon as the fire is in the dispatcher's channel; dispatch is asynchronous. Errors: `401` wrong/missing secret, `404` unknown or disabled trigger, `413` payload over 256 KB, `429` over 60 requests/minute (token bucket per trigger), `503` dispatcher backed up. The web port is only reachable from other machines with `--server`.
- **Device** requests fire every enabled device trigger for that device whose secret matches and whose `events` list includes the event (empty = all). Runs are queued even if the agent's container is stopped.
- **Event** triggers read the broadcast bus. `agent.turn_done` payloads carry `agent_id, run_id, status, error, source_type, source_id`; filters compare these fields (snake_case). A trigger never fires on a run it dispatched itself (the fire is recorded as `skipped`), so "retry on failure" rules can't loop on their own output.

## Calendar backups

Each day also has a backup: the stand-in's `backup`, else the calendar's `backup`, else the next person in the turns, and never the primary. What a Nandi does at a scheduled time depends on whether its owner's names appear in the calendar (`triggers::calendar::role`). Those names are its instance ID, display name, owner name, handle and user slug.

| Role                             | At the time                                                      |
| -------------------------------- | ---------------------------------------------------------------- |
| Owner (not named)                | Fires for the primary, then watches that run in its own database |
| Primary (named, their day)       | Fires; the backup's Nandi does the watching                      |
| Backup (named, their backup day) | Waits 5 min, then asks the primary's Nandi over the relay        |
| Bystander (named, not today)     | Nothing                                                          |

So one trigger file can sit in everyone's repo, and whoever is on duty runs it.

**How a watch decides** (`triggers::handover`). It asks how the duty is going until the duty settles:

- The answer comes from the duty's `trigger_fires` row (matched on `payload.duty_id` and `role`) and its run's status, so it survives restarts.
- It hands over when the run failed, never started, or is still going `backupAfterMinutes` after the fire.
- Over the relay it also hands over when the primary's Nandi is offline or doesn't answer `DutyQuery` within 15 s.
- If the primary's clock is behind (`Upcoming`), it asks again 5 min after the run is due.

**Handing over** fires the trigger again with `role: backup`, `responsible` set to the backup, and `primary` and `handover_reason` added. That fire skips the rate limit.

**Limits.** A Nandi answers a `DutyQuery` only when the asking member is named in that trigger's calendar. Watches live in memory: a restart cuts one short and it is not resumed, and a watch gives up a day after the scheduled time.

`DutyQuery` and `DutyStatus` are new `PeerMessage` variants, so the relay must be rebuilt from the same version.

## Dispatch

Fires are processed one at a time (the dispatcher task and manual test fires share a lock):

1. **Storm guard.** More than 5 fires in 10 seconds disables the trigger and emits `trigger://error`. Cron, calendar and test fires are exempt, so an every-second schedule is allowed.
2. **Rate limit.** If `rate_limit_seconds` hasn't passed since the last dispatched fire, the fire is recorded as `rate_limited` and `trigger://rate_limited {nextAllowedAt}` is emitted.
3. **Template.** `input_template` is rendered: `{{payload.a.b}}` (array indices allowed), `{{trigger.name}}`, `{{trigger.id}}`, `{{fired_at}}` (ISO 8601), `{{last_run.output}}` (the latest completed run's reply). Missing keys become empty strings and are logged. With no template, the agent gets the event as JSON.
4. **DB first.** The `trigger_fires` row is written (`queued`, no run yet), then the run is submitted with `source_type` = the trigger's type, `source_id` = the trigger ID, `source_label` = its name, and its priority. The row is then updated with the run ID, or set to `error`.
5. `trigger://fired` (or `trigger://error`) is emitted, and the fire is appended to `~/.nandi/local/logs/triggers.log` (metadata only, never the payload).

Workflow targets start `run_workflow` and record the workflow run's ID. Workflows don't take input yet, so the template is ignored for them.

## Storage

Rules are files: `~/.nandi/repo/.nandi/triggers/<id>.yaml` (format below, validated by `domain::trigger`). The `triggers` table is their projection, `trigger_state` holds `last_fired_at`, and `trigger_fires` the fire history; see [data-model.md](./data-model.md). Every change (create, update, enable/disable, delete, rotate secret) writes the file through `TriggerRepository` (atomically, staged in git), commits it to the projection (`repo://trigger-*` + `trigger://changed`), and reloads the rules, re-registering cron jobs and watchers right away. Editing a file by hand, or pulling one, does the same through the file watcher.

```yaml
id: daily-digest
name: Daily AI News Digest
enabled: true
source:
  type: cron
  config:
    schedule: '0 9 * * 1-5' # 9am weekdays
target:
  type: agent
  id: research-agent
input_template: |
  Find AI infrastructure news from the last 24h and summarize the top 5.
priority: 80
rate_limit_seconds: 3600
```

**Secrets live in the OS keyring**, under the trigger's `secretRef` (default `nandi.trigger.<id>`), never in the file or SQLite. They're generated (256 bits) on save, returned once (`SavedTrigger.secret`, on create, on a change to a webhook/device source, and on `rotate_trigger_secret`), attached in memory when the rules load, stripped from every listing (`Trigger.hasSecret` says one exists), and never logged. A file with a `secret` key in its config is rejected. A trigger loaded from the repo on a new machine gets a fresh secret.

## API

Tauri: `list_triggers`, `get_trigger`, `create_trigger`, `update_trigger`, `delete_trigger`, `set_trigger_enabled`, `rotate_trigger_secret`, `test_fire_trigger` (works while disabled, skips the storm guard and rate limit), `list_trigger_fires`, `get_web_origin`, `preview_cron`, `preview_calendar` (the next 5 runs with who is on duty).

HTTP: `GET/POST /api/triggers` · `POST /api/triggers/cron-preview` · `POST /api/triggers/calendar-preview` · `GET/PUT/DELETE /api/triggers/{id}` · `POST /api/triggers/{id}/enable|disable|secret|test` · `GET /api/triggers/{id}/fires?limit=` · receivers `POST /api/triggers/webhook/{id}` and `POST /api/triggers/device/{deviceId}`.

```sh
curl -X POST http://127.0.0.1:17463/api/triggers/webhook/new-orders \
  -H "X-Trigger-Secret: $SECRET" -H "Content-Type: application/json" \
  -d '{"order": {"id": 42}}'
```

## Frontend

- `src/hooks/useTriggers.ts`: TanStack queries (`['triggers']`, `['trigger', id]`, `['triggerFires', id]`). `agent-sync` invalidates them on `trigger://*`, so lists, history, and the dashboard update live.
- `src/lib/trigger-form.ts`: form state ⇄ `TriggerInput` (unit-tested). `src/lib/trigger-actions.ts`: user actions.
- Views: `TriggersListView` (filter chips, enable toggles, test fire), `TriggerDetailView` (Definition / Fires / Payload tabs), `NewTriggerDialog` (a tab per source; shows a new secret once). The sidebar lists triggers by source type; the dashboard has a Recent triggers card with enabled / fired-today counts.
