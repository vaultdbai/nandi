# Live Work Happens in Chats

**From:** owner, 2026-11-25, testing 0.3.1: "I don't have ability to use current runs as chats and work with them, and approvals go to some other screen for me to approve. There is no better way to monitor the active run. Historical viewing is good. ... Maybe active runs are nothing but chats in Ask for different agents playing a part in them, and the default agent is Nandi."
**Serves:** trust (every approval answered where the work is, in context), state (one place per piece of live work: its chat, kept with its run), verification (the person watches each step, the plan and the result live).

## Today

| Work                             | Where it shows                                  | Approve in                    | Reply? |
| -------------------------------- | ----------------------------------------------- | ----------------------------- | ------ |
| Chat with Nandi                  | Its chat (one conversation, many runs)          | The chat                      | Yes    |
| A trigger's or a workflow's run  | Run Detail, the Inbox                           | Inbox, Run Detail › Approvals | No     |
| Nandi delegates to another agent | A separate task thread (`DelegationThreadView`) | That thread, Inbox            | No     |
| An ask from the phone            | Its own conversation                            | Phone, Inbox                  | No     |

## The Model

- **Live work is a chat.** Every running or waiting run, however it started, shows in Ask's chat list under **Live**, with its agent's name and how it started ("Morning brief", "From your phone", "Workflow: Pay bills").
- **Nandi is the default agent.** A new chat is Nandi's. When Nandi brings in another agent, that agent's steps and approvals show **inside the same chat**, in a block named for it ("Researcher is working on it").
- **In the chat:** the messages as they stream, the plan card, approval cards, Stop, and a reply box (a reply is the run's next turn).
- **Runs tab:** history only. A live run there says "Live: open the chat".
- **Inbox:** still lists what waits for the person, each item opening its chat.

## Design

**From:** owner, 2026-11-25: "think deep about run usability ... best usability".

The person never needs the word "run". They see Nandi working in a chat. Every live chat answers three questions at a glance: what is it doing now, does it need me, did it work.

| #   | Principle                     | What it means                                                                                                                                         |
| --- | ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | Needs you first               | Live sorts: waiting for the person first, marked "Needs you"; then working; then finished in the last hour (faded). No live work: no section at all   |
| 2   | One human status line         | Per live item and at the top of the open chat: the current plan step's title, else the agent's latest short message, never a tool name. "Step 2 of 5" |
| 3   | Steps quiet, decisions loud   | Tool calls collapse under "Show steps". Messages to the person and approval cards stay full width. The approval card keeps its preview and Yes / No   |
| 4   | Steer while it works          | The reply box is there while it works. A message sent then shows "<Agent> will read this next" and becomes the next turn (queued, the engine as is)   |
| 5   | Stop is one tap               | At the top of a live chat, never in a menu                                                                                                            |
| 6   | Other agents read like people | Their name and icon on their block ("Researcher is working on it"), their approvals in the same chat                                                  |
| 7   | Calm failure                  | A failed run says what happened in one line and offers Try again or "Tell <Agent> what to change". Details stay under "Show steps"                    |

## One Nandi

**From:** owner, 2026-11-25: "if Ask is always going to use some agent or Nandi by default then why do we need nandi in the left side or ask ... click on ask nandi shows what nandi has now and new chats start as runs and nandi's active runs show up there as chats and older runs (after 5 or 10) shows up in runs ... how will we create new agents or do other things ask does? maybe nandi does all that too".

One place to talk: Nandi. Ask and the Nandi item become one. Nandi makes agents and automations too, and another agent is one `@` away.

## Slices

1. **Live runs in Ask.** (done) A Live section lists every running or waiting run; opening one shows its messages, plan card, approval cards and Stop. Reuses `MessageList`, `ApprovalRequestCard` and `StopButton`. No new storage: it reads `runs.conversation_id`. Built: design points 1, 2, 3, 5 and 7 (Try again); the sidebar lists the live chats under Ask. No run starts from a phone today (a paired phone only approves), so there is no "From your phone" origin yet.
2. **Reply into a live run.** (done) The reply box sends the next turn in that run's conversation (triggered, workflow and phone runs included). Built: design point 4 (it waits in the agent's queue, "<Agent> will read this next") and 7 ("Tell <Agent> what to change").
3. **Delegated work inside the parent chat.** (done) The child run's steps and approval cards show in a block in the parent chat; approving there answers the child's approval. Built: design point 6. The link is the parent's agent and the parent run's time span (`list_delegated_runs`), with no new storage: an agent runs one turn at a time and its tool calls in order.
4. **Inbox and Runs point to the chat.** (done) An Inbox approval opens its chat; a live run in the Runs tab links to it.
5. **LATER: the receipt.** At the end of a finished chat run: what was done, what changed, proof links (screenshots, sent items), and "Do this automatically", which drafts a workflow or trigger from the chat for the person's yes.
6. **LATER: one look.** The app icon, the tray and the phone show the same three states: calm, working, needs you.
7. **The Nandi home.** (done) One menu item **Nandi** (the sitting bison) replaces Nandi and Ask: a composer ("What should I do?"), then **Live**, then **Recent** (the last 10 chats, newest first, pinned ones kept; older ones in History). A new chat is a new conversation for Nandi; the long `agent:nandi` conversation stays as one chat. Studio's editor stays reachable from an agent's page as **Edit**. Built: the box is pinned at the bottom with the example asks above it (owner, 2026-11-26); Nandi's chats show the sitting bison and plain-text previews; pins are kept in this computer's UI storage (no migration); a chat is a `conversations` row (`chat:<id>`), so no schema change.
8. **Nandi creates agents and automations.** (done) "Make me an agent that …" or "every Monday …" in a Nandi chat shows a preview card from the planner Ask uses (`services/ask/planner.rs`): what it will create, what it may touch, **Create / Edit / No**. Create goes through Ask's save path; Edit opens the editor. A tool returns the plan; the model does the understanding. Built: Nandi gets Ask's guides, check and save tools in the person's chats; `preview_save` reads the call's file for the card; the save stays desktop-only for a phone.
9. **@mention another agent.** (done) `@` in the composer lists your agents; `@Researcher …` starts or continues that chat with Researcher. Those chats show in Live and Recent with its name and icon.
10. **The menu, cut to what people come for:** (done) Nandi, Inbox, Automations (workflows and triggers, each with its last and next run), Agents, History (past chats and runs, searchable, the audit log from it), then Library, Network, Settings. Existing screens move under these. Built: Home's page is the Inbox page (its Ask box removed: Nandi is where you talk); History is All activity with past chats above it and the audit log beside; Automations keeps its two tabs, each list with a Next run column.

Each slice lands on its own, with its entry in `docs/testing/agents-chat-runs.md` ("Needs relay: no").
