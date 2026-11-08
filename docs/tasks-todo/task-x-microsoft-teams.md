# Task: Microsoft Teams

**From:** owner, 2026-11-07. Levels 1 and 2 first; level 3 later.
**Size:** M for levels 1 and 2 (the same Entra app and the same intake as email); L for level 3.
**Builds on:** the Outlook connector and its Entra app (`services/email/`, `services/auth/oauth.rs`), the calendar check pattern (`services/workspace/`), intake (`services/intake/`: screening, the queue, threads, auto-approve rules).
**Rule:** read-only, like email ("read and draft, never send", owner 2026-11-07). Nandi never posts in Teams; drafting a reply for the user to send is a later step.

## Level 1: Teams as a source

- [ ] **Sign-in:** add `Chat.Read` (user consent) to the Microsoft sign-in; channels the user picks need `ChannelMessage.Read.All` (admin consent), so channels are optional and say so. The owner adds them on the Entra app (client `c62fc612-…`).
- [ ] **A Teams check** next to the email and calendar checks: reads new chat messages (and picked channels) since a cursor (Graph change tracking where available, else polling `/me/chats/getAllMessages` delta), filters out the user's own messages, bots and notifications, and hands one batch to the Chief of Staff in its own conversation (`teams:<check id>`), like `email::check_all`.
- [ ] **Source `teams`** (`RequestSource::Teams`): the Chief of Staff notes asks, promises, decisions and owners, and what needs doing goes through intake (screened, queued, a thread per item, auto-approve rules can match a person by their Entra user ID, which Microsoft verifies).
- [ ] **Tools:** `teams_list_new`, `teams_get_message`, `teams_get_chat` (read-only, untrusted data, like the mail tools).
- [ ] Settings › Email & calendar becomes "Microsoft 365" or gains a Teams row: on/off, which channels, how often; Pause and Disconnect as for mail.

## Level 2: Meetings

- [ ] After a Teams meeting the user attended ends, read its **transcript** (`OnlineMeetingTranscript.Read.All`, admin consent; the meeting must have been transcribed) and recap, and hand it to the Chief of Staff: decisions, owners, deadlines and follow-ups into the notes (`meetings.md`), and to-dos through intake.
- [ ] Link the meeting to its calendar event (the calendar check already sees it), so prep notes and the recap sit together.
- [ ] No transcript (not recorded, or no consent): say so once on the event; never record or join meetings.

## Level 3 (later): talk to Nandi in Teams

- [ ] A Teams app with a bot, installed by the company, so the user messages their Chief of Staff from Teams. Messages reach the user's own Nandi through the relay (sealed), like the phone apps (workspace `tasks/tasks-todo/task-x-mobile-apps.md`). Needs relay: yes. Approvals still need the user on a paired device.

## Tests

- A batch of chat messages reaches the Chief of Staff once; own messages, bots and notifications are filtered; the cursor survives a restart.
- A Teams ask becomes a queued item with a thread; an auto-approve rule for that person's Entra ID lets a clean one through.
- A transcript becomes a recap in the notes and to-dos in the queue; no transcript is handled calmly.
- Without the Teams scopes, nothing changes for mail and calendar.
