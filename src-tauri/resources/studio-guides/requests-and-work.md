---
id: requests-and-work
title: Requests and Work
when: Work in progress, run now, pause, cancel, rate.
---

## What a Request is

A Request is one piece of work with steps: messaging an agent, running a workflow, asking a person on the network, or a manual step. Work others send the user is a Request; so are the user's own asks once they run. Each Request has a page that shows where it is, who has to act, and why it's waiting. It picks up where it was after a restart.

## The Work view

Work lists Requests in flight and finished, with what each is waiting for. Home shows the same in short: the Inbox (what needs the user), Running and Recent. From a Request the user can:

- **Pause** and **Resume** it.
- **Cancel** it. This asks first.
- **Mark done** a step that waits on them.
- **Rate** a finished delegation once.
- **Do this again**, or **Save as workflow** to keep the steps.

## Doing things from chat

- "Run X", "do Y for me", "every Monday…": start a Request with `nandi_start_request`. Give it the user's words; it builds a plan from the agents, workflows and people this Nandi knows. The user sees the plan (steps, targets, time, cost, warnings) and approves it before anything starts. Then link the Request in your reply.
- "Every Monday…" means a schedule. Read the set-up-a-trigger guide and propose a trigger, or start the Request now and offer the trigger as well.
- Asking one person or one agent directly: use `delegate_to_agent`.
- Don't claim something ran unless the tool said so.

## Answering about work

1. Call `nandi_list_requests` for what's in flight and finished today.
2. Call `nandi_get_request` for one Request's steps and why it waits.
3. Call `nandi_list_runs` for agent runs; never say you can't see history.
4. Say who has to act and what button fixes it.
