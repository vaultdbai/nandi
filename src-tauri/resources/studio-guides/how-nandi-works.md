---
id: how-nandi-works
title: How Nandi works behind the scenes
when: What runs on its own; how Chief of Staff tracks work and notes.
---

Use this for "how does…", "when does…" and "does it keep watching…" questions. Answer plainly, and say what doesn't happen as clearly as what does.

## Nothing runs unless something starts it

An agent runs only when one of these starts it:

- the user sends it a message (its chat, Ask, the companion)
- a Request or a workflow step
- a trigger fires (a schedule, a file change, a webhook, a device or another run finishing)
- someone on the network asks it (an `ask-me` question, `assign-me` work, or a call to an agent the user shares)

Between runs, no agent watches anything, and no model is called. Triggers fire only while Nandi is running. Requests and assigned work pick up where they were after a restart.

## How Chief of Staff keeps track of work

It keeps no list of its own. When asked "what's open?" or "what am I waiting on?", it calls `my_requests`, which reads the app's records of requests both ways (asked by the user, asked of the user), with status, due date and what's overdue. A teammate's Done or Decline is recorded by the app as soon as it arrives, even while Chief of Staff isn't running, and shows up the next time it's asked. The arrival doesn't start Chief of Staff, so it can't tell the user by itself. Work the user assigns themselves isn't listed by `my_requests`; it's on their Inbox.

Who's responsible for what comes live from the network (`find_people`), not from notes.

## When Chief of Staff updates the notes

The model decides during a conversation with the user, guided by its skill.md. No app code chooses for it.

- **Getting started:** if the notes are empty or thin, it offers once to ask a few questions, then writes the first files.
- **Noting as you go:** when the user mentions a deadline, decision, owner change, contact or commitment, or a request's result comes up in the chat, it writes the right file and says so in one line ("Noted in deadlines.md").
- **Work others assign the user:** early in each chat it checks `my_requests` for what's been asked of the user, and notes what's new on their list (who asked, what, due) and what got done or declined. Work waiting for the user's approval is in their Inbox.
- **Thinking ahead:** while it's talking with the user, it raises deadlines this week, overdue items and notes that disagree.
- **Never** while answering someone else's `ask-me` question: it reads the notes then, but doesn't change them. Ask itself doesn't write notes either.

It can write only when **Let Chief of Staff keep them up to date** is on (My notes). Then writes into `my-notes/` don't ask; with it off, the folder is read-only to agents. So if the user never talks to it, the notes don't change and nothing gets flagged.

## Making it check in on its own

To have it look ahead without being asked, suggest a schedule trigger that messages Chief of Staff, for example at 09:00 on weekdays: "Check my deadlines and open requests and tell me what needs my attention today." Read the set-up-a-trigger guide. Its reply is in the trigger's Firings and the run. Notes writes still don't ask, but asking or assigning anyone still waits for the user's approval.

## Other things people ask

- **Updates:** packs and agents update only when the user chooses Library › Installed › Check for updates. Nothing updates on its own.
- **Where agents work:** each agent's tools run in its own container; see the runtimes-and-sign-in guide.
- **What teammates see:** what the user publishes in Network › Sharing (responsibilities, profile, shared agents). Notes are quoted only in answers to questions, and never from `my-notes/private/`.
- **Approvals:** risky tool calls, requests to other people and new installs ask first; see the approvals-and-policies guide.
