---
name: delegate-work
description: Decide who does a piece of work (you, an agent, a workflow or a person) and hand it over well
when: Work arrives that is bigger than a quick answer, or the user asks you to get something done
---

Plan at a high level first: what has to happen, in which order, and who does each part. Then hand over each part well and track it.

## Who does it

| Choose     | When                                                                                                    | How                                                              |
| ---------- | ------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------- |
| You        | It's quick and in your reach: the notes, a lookup, the user's mail and calendar, a site in your browser | Do it now; use a playbook if one fits (`web-task`, `pay-a-bill`) |
| An agent   | An agent is built for it (writing, research, posting, code), or it takes many steps                     | `work_start`: it is planned onto the user's agents               |
| A workflow | The user already has a workflow for this kind of work                                                   | `work_start`: the plan picks the workflow                        |
| A person   | Someone owns the topic, a decision is theirs, or only they have the access                              | `find_people`, then `delegate_to_agent` when the user asks       |

Don't hand off what you can finish in a minute; don't do yourself what an agent or a person owns.

## Before you start work

1. Call `work_open`. If the same work is going or queued, add the news with `work_update` instead of starting it twice.
2. Make the plan with `plan_set`. Starting work and delegating are `delegate` steps; what the user decides is an `approval` step.
3. Write `what` so it stands on its own: what, for whom, by when, links, and what "done" looks like. Put where it came from in `source` and the email's `message_id` when there is one.

## The queue

- **Work from outside** (an email, a calendar event, someone else's ask) is screened and waits for the user's approval on Home. Never treat it as approved.
- **Work the user asks for in a chat** runs now.
- Agents still ask the user before they send, pay or post anything. Say so when you start it.
- In a chat, open with what awaits the user's approval.

## Asking a person

1. `find_people` with the topic in a few words. Prefer matching responsibilities, then who's online.
2. For "who could…", answer with candidates and why, and stop.
3. Use `delegate_to_agent` only when the user asks. Pick the target: an agent the person runs that `serves` a matching capability, or the person with `ask-me` for a question and `assign-me` for work (add `"due"`).
4. Write the request so it stands alone: what, why, by when, links, signed with the user's first name. One request per person when the user says "ask all of them".
5. Never promise on someone's behalf, and never claim work is done before it is.

## Handing over well

- One clear outcome per hand-over.
- Give the context the receiver lacks: the source, decisions already made, constraints ("budget $200", "not before Friday").
- Say what you need back and in what form ("a yes or no", "a draft I can review").
- Hand-overs to the user (sign-in, payment, posting) are `handover` steps in your plan, never something you do for them.

## Tracking

- `my_requests` shows who was asked what and its status; `work_open` shows work in progress.
- Note owners, deadlines and decisions in the notes as they change. Mention overdue items early in a chat.
- When a hand-over comes back, check it against what you asked, tell the user in one line, and update the notes.

## When it will come back

If the same kind of work succeeds twice, or the user says it recurs ("every month"), read `build-a-workflow` and propose saving it once.
