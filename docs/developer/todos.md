# To-dos and Reminders

The user's to-dos are **assignments to the user**, not a second list. They reuse `gateway_assignments`, the Inbox `assignment` kind and its card, its Done, and its Ask chat (`inbox:assignment:<id>`). Needs relay: no.

## The Row

A to-do is a `gateway_assignments` row with a `source` (migration `008_todos.sql`):

| Column                     | To-do                                                         | Work a person assigned |
| -------------------------- | ------------------------------------------------------------- | ---------------------- |
| `task_id`                  | `todo-<uuid>` (`gateway::todos::ID_PREFIX`)                   | the gateway task ID    |
| `caller_instance`          | empty: no remote caller                                       | their instance         |
| `caller_name`              | who added it (Nandi's name, or the user's)                    | who assigned it        |
| `request`                  | one plain sentence with the deadline                          | what they asked        |
| `kind`                     | `do`, `check` or `reminder` (`domain::todo::TodoKind`)        | `do`                   |
| `source`                   | `email`, `event`, `chat` or `manual`                          | `NULL`                 |
| `source_ref`               | where it was read (an email's message ID); unique per source  | `NULL`                 |
| `related_ref`              | an Inbox thread Nandi prepared for it (`inbox:approval:<id>`) | `NULL`                 |
| `run_id`, `assigned_agent` | the run of the agent the user handed it to                    | `NULL`                 |

`GatewayAssignment::is_local()` is `source.is_some()`. `run_status` is read from `runs` on every select, never stored.

## Rules

- **Idempotent add:** `assignments::open` returns the row already there for the same `source` + `source_ref` (`by_source`), so a mail read twice is one to-do.
- **Nothing reaches the relay:** closing a local one skips the gateway waiter (`close_assignment_as`); there is no gateway task to answer. Reassign is never offered in its Ask chat (`inbox::item`, Done only).
- **Hand-off:** `todos::hand_off` sends "Please do this: <text>" to the agent's chat (Nandi: the to-do's Ask chat) and stores the run. The UI shows it under Working while the run is active (`todo-model.ts` `todoState`), then "finished: mark done?". Only Nandi closes one itself, with `todo_done`.
- **Reopen (Undo):** `assignments::reopen` works on done to-dos only and clears the note, the close time and the hand-off.
- **Record:** `add` (`added`), `hand_off` (`handed to <agent>`), `done` (`done`) and `reopen` (`reopened`) each write one Decisions entry, kind `assignment`; an agent's under its name (`Decided::by`).

## Nandi's Tools

`gateway::todos::tool`: `todo_add {text, kind, due?, source, source_ref?, related_ref?}`, `todo_list {}`, `todo_done {id, note?}`. Host tools, offered to Nandi only when its `skill.md` lists them in `allowed-tools`, allowed in the email and calendar check runs (`data_run_allows`), no approval (`run_plan::classify`: no effect). What to add is in Nandi's `skill.md` ("To-dos"), the `email-triage` playbook and the `mail-check` and `morning-digest` triggers, not in code.

## The UI

| Piece                                     | Where                                                                    |
| ----------------------------------------- | ------------------------------------------------------------------------ |
| States, tones, badge, chip, groups, links | `src/components/inbox/todo-model.ts` (pure, tested)                      |
| The card                                  | `Inbox.tsx` `AssignmentRow` (one card for both)                          |
| Marker, hand-off menu, Add, Done recently | `src/components/inbox/TodoParts.tsx`                                     |
| Tray rows                                 | `src/components/tray/tray-model.ts` (`todoTrayRow`, Working, Done)       |
| Data                                      | `useTodos`, `useAddTodo`, `useReopenTodo`, `useHandTodo` (`useInbox.ts`) |

Colours are theme tokens only (`TODO_TONE_TEXT`): overdue `destructive`, due within 2 days `warning`, Do `primary`, Check `info`, a reminder `highlight` with a bell. A reminder counts in the badge only within 3 days of its date or past it; the list always shows it.
