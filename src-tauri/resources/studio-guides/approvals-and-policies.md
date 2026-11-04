---
id: approvals-and-policies
title: Approvals and policies
when: Approvals, safer agents, rules for others' asks.
---

Nandi has approvals per tool call, per workflow step, and per incoming request, plus written policies.

## Per tool call

- Tools listed in an agent's `requires_approval` wait for the user before every call. The card says what the call will do in plain words and shows the arguments under Details.
- An approval nobody decides expires after an hour and counts as a reject. The model is told and should change its plan, not retry.
- The run is suspended while it waits: no tokens, and it survives a restart.
- Approvals apply on every runtime. On a CLI runtime the CLI waits while the call is in the Inbox, and a run that stops while waiting is cancelled.
- Good default for a new agent: put `shell` and `write_file` (and any tool that sends or posts) in `requires_approval`. Relax it once the user trusts the agent.

## Per workflow step

A workflow step can ask first. The run pauses before that step's message is sent. Put this on the step that acts on the outside world.

## Per incoming request

Other Nandis can ask the user's agents for work. For each capability the user can allow it, hold it for approval, or hide it. Held requests show in the Inbox with Approve and Decline.

## Consent

- Installed agents can't run until the user allows each thing they ask for (tools, network hosts, AWS, pilots).
- Pilots (control of the user's computer) need a grant per agent and per capability, and ask per call unless read-only.

## Written policies

- `NANDI.md` in the repo describes this Nandi's gateway: what it offers, trusted peers, and rules for callers.
- `.nandi/policies/*.md` hold rules the app enforces, one topic per file. They only take away: an agent gets what its skill.md asks for, narrowed by every policy that applies, and a deny anywhere wins.
- These are files the user owns. Propose changes as text; the user applies them. Never loosen a policy without saying exactly what it would allow.

```markdown
---
applies_to: [agent:report-*, workflow:month-end, tag:finance] # or ['*']
tools: { allow: [read_file, shell], deny: [shell], ask: [write_file] }
files: [{ path: 'D:/Finance', mode: read }, { folder: reports, mode: read }]
network: { allow: [{ host: '*.amazonaws.com' }] }
aws: { services: [{ service: s3, actions: ['Get*', 'List*'] }] }
models: { allow: ['claude:*'] } # Nandi's provider:model names
connectors: { allow: [github], run_on_host: [] }
pilots: { allow: [] }
---

Why this exists, in plain words. The text never grants anything.
```

- Without `applies_to` a file is notes only; a file with rules and no `applies_to` is refused.
- A file that doesn't parse stops every agent it may cover, so check it with `nandi_check_artifact` before showing it.
- The agent page's Permissions section shows which policies apply to an agent and what each changed.

## Steps

1. To answer "what's waiting on me", call `nandi_list_requests` and look at the Inbox items.
2. To tighten an agent, call `nandi_get_agent`, propose a new `requires_approval` list, run `nandi_check_artifact`, then show the file.
