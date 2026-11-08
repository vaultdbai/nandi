---
name: build-a-workflow
description: Spot work that recurs and help the user save it as a workflow that runs the same steps every time
when: The same kind of work has gone well twice, or the user says it repeats (every week, every month)
---

A workflow is saved steps that run the same way every time, by hand or on a schedule. You don't write workflows yourself; you notice the pattern, propose it once, and get the user to the right place.

## When to propose one

- The same kind of work succeeded twice (the same bill each month, the same weekly report).
- The user says it repeats: "every Monday", "each quarter", "whenever a new hire starts".
- A Request finished well and its steps would work again with only the dates changed.

Don't propose for one-off work, work that needs judgement at every step, or something the user dismissed before (check `my-notes/suggestions.md`).

## Nandi proposes finished work for you

When a Request finishes well and the same work (or the same plan) finished well before, Nandi adds your proposal to the task's thread and to Home's "Since you were away", with a **Save as workflow** button. Don't repeat it in chat; answer questions about it. The draft turns the hand-overs of your run plan into manual steps for the user, and nothing is saved until the user checks the draft and saves it.

## How to propose

1. Say it in one or two lines: what would repeat, when it would run, and what the user still does.
   "Paying the Acme bill went well twice. Save it as a workflow that runs on the 5th of each month? You'd still sign in and press Pay."
2. Note the proposal once in `my-notes/suggestions.md` with the date. Never repeat a dismissed one.
3. If the user agrees, point them to the right place:
   - **From finished work:** press **Save as workflow** on the proposal, or open the Request on Work and press it there. It drafts a workflow from the Request's steps; the user checks the file and saves.
   - **Something new:** ask Studio (Ask) to build it, in the user's words: what triggers it, the steps, who does each step.
4. Add a schedule only if the user wants one. Studio sets up the trigger.

## What makes a good workflow

- Each step has one owner: an agent, a person, or the user.
- The user's parts stay manual steps: signing in, paying, posting, deciding. A run plan's hand-overs become these manual steps.
- Inputs that change each time (an amount, a date, a recipient) are named as inputs, not written into the steps.
- It says where the result goes: a note, a message, a file.

## After it exists

- When the work comes again, `work_start` picks the workflow; you don't redo it by hand.
- If a run of it fails or the steps drift from how the user works, tell the user once and suggest the change in Studio.
- Record in the notes which recurring work has a workflow, so you don't propose it again.
