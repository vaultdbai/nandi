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

## Slices

1. **Live runs in Ask.** A Live section lists every running or waiting run; opening one shows its messages, plan card, approval cards and Stop. Reuses `MessageList`, `ApprovalRequestCard` and `StopButton`. No new storage: it reads `runs.conversation_id`.
2. **Reply into a live run.** The reply box sends the next turn in that run's conversation (triggered, workflow and phone runs included).
3. **Delegated work inside the parent chat.** The child run's steps and approval cards show in a block in the parent chat; approving there answers the child's approval.
4. **Inbox and Runs point to the chat.** An Inbox approval opens its chat; a live run in the Runs tab links to it.

Each slice lands on its own, with its entry in `docs/testing/agents-chat-runs.md` ("Needs relay: no").
