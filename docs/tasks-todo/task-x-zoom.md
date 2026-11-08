# Task: Zoom (low priority)

**From:** owner, 2026-11-07. Low priority: after Microsoft Teams (`task-x-microsoft-teams.md`). Meetings first, then Team Chat.
**Size:** M.
**Rule:** read-only, like email and Teams ("read and draft, never send"). Nandi never records, joins or posts.
**Builds on:** the same pattern as Teams: a check per source, the Chief of Staff's own conversation per batch, intake (screening, the queue, threads, auto-approve rules).

## Setup

- [ ] A Zoom app in the Zoom App Marketplace (OAuth, user-level), with its own sign-in in Settings: tokens in the keyring, refresh as for mail. Check the current scope names at build time (cloud recordings and transcripts, meeting summaries, Team Chat messages, all read-only).
- [ ] Publishing to other companies needs Zoom's app review; each company's Zoom admin may need to allow the app.

## 1. Meetings first

- [ ] After a meeting the user hosted or attended ends, read its **cloud recording transcript** and, where AI Companion is on, its **meeting summary**, and hand them to the Chief of Staff: decisions, owners, deadlines and follow-ups into `meetings.md`, to-dos through intake.
- [ ] Find ended meetings by polling (Zoom's "recording completed" webhooks need a public address; the relay could carry them later).
- [ ] Link the meeting to its calendar event, as for Teams.
- [ ] No transcript (local recording, no cloud recording on the plan, AI Companion off): say so once on the event.

## 2. Team Chat

- [ ] Zoom Team Chat messages as a source, like Teams chat: notes, and to-dos into the queue. Auto-approve rules can match a person by their Zoom user ID.

## 3. Later

- [ ] A Team Chat chatbot or Zoom App to talk to Nandi from Zoom, through the relay, like level 3 of the Teams task.

## Tests

- A transcript and a summary become a recap and queued to-dos; a meeting without a transcript is handled calmly.
- Team Chat messages reach the Chief of Staff once; the cursor survives a restart.
- Without a Zoom sign-in, nothing else changes.
