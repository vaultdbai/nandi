---
id: set-up-a-trigger
title: Set up a trigger
when: Schedules, file changes, webhooks and events.
---

A trigger starts one agent run or one workflow run when something happens. It's a YAML file in the user's repo, and it can be switched on and off.

## Sources

- **Schedule**: a cron expression, in the user's local time. `0 9 * * 1-5` is 9:00 on weekdays.
- **File**: a file in one watched folder is created, changed or deleted, optionally filtered by name patterns. The folder must exist.
- **Webhook**: something POSTs to the trigger's local URL with its secret. The secret lives in the keychain, never in the file.
- **Device**: a device POSTs an event for its device ID.
- **Event**: an agent run finishes, or a workflow finishes or fails. A trigger never fires on a run it started itself.

## What it starts

The target is an agent or a workflow. The message it sends comes from a template that can use the event's payload (for a file, the path that changed), the trigger's name and the time. With no template, the agent gets the event as JSON.

## Steps

1. Make sure the agent or workflow exists (`nandi_list_agents`, `nandi_list_workflows`). If not, build it first and say so.
2. Check what already fires: `nandi_list_triggers`.
3. Pick the source from what the user said. "Every Monday" is a schedule; "when I drop a PDF in Inbox" is a file trigger.
4. Read the schedule back in words ("At 09:00, Monday through Friday") so the user can confirm it.
5. Keep a rate limit on noisy sources. A trigger that fires more than 5 times in 10 seconds is switched off automatically.
6. Save it with `nandi_save_artifact` (kind `trigger`, after the agent or workflow it starts is saved). The user approves the save; they can also build it in Automations › Triggers.
7. Tell the user about **Test…** on the trigger's page: it really runs the target, so it asks first.

## Judgement

- A trigger runs without anyone watching. Make sure the agent's risky tools still ask for approval, or say plainly that they won't.
- For "do this once, now", don't make a trigger. Start a Request instead.
