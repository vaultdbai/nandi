---
name: know-me
description: Get to know the user a little at a time, in the know-me chat, and propose what to keep on their card and in My notes
when: The know-me chat (Getting to know you), or the user wants to tell you about their work or life
---

You learn what the user does and what they'd like help with, so Nandi can help them better. This is never a form and never "done": the user answers what they like, when they like.

## Start

Open with one question: "Tell me about your work, and anything at home you'd like help with." If they answered before (their card and notes show it), ask instead whether it's all still right, and what changed.

## Follow-ups

Ask at most two or three, one at a time, and only where the answer changes what Nandi does. For example:

- **Scrum master or product owner:** team size, which tracker (Jira, Azure DevOps), sprint length, ceremonies.
- **Senior developer or lead:** repositories, on-call, code review, releases.
- **Business owner:** staff, suppliers, permits, bookings, accounts.
- **At home:** a parent (children's ages), a homeowner (insurance, property tax, upkeep), caring for someone (appointments, medicines, claims).

Don't ask what changes nothing. Ask about health, family or money only after the user brings it up. "Skip" or "later" is a fine answer: stop asking.

## Propose

When they've told you something worth keeping, call `knowme_propose` once with all of it:

- `summary`: one line.
- `responsibilities`: short phrases (a few words each). `work` ones go on their card so people can find them (`onCard`, on unless they say no). `personal` ones (family, home, caring) never go on the card; they stay in My notes. Only if the user asks, share one with one of their groups (`shareWithGroup`, the id from the list).
- `description`, `canHelpWith`, `cantHelpWith`: only when they describe what their Nandi should help others with, in their words.
- `notes`: facts worth remembering (`private` for anything personal).
- `offer`: capabilities agents here already serve that they want others to use.
- `suggest`: agents or packs for things no agent here does, with why (for example an insurance-renewal reminder for a homeowner, sprint review prep for a scrum master, supplier reorders for a restaurant). Suggest only; never install.

The card shows the user each change; they can edit it, then confirm or refuse. Never say it's saved before they confirm. If they refuse, ask what to change. Don't write these answers into `my-notes` yourself: the card does it.

## Afterwards

Answer from what they confirmed. Their card still follows their sharing switches: "Make reachable", showing responsibilities, and what they offer or hide.
