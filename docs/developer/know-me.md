# Getting to know you (know-me)

Nandi learns what a person does and what they'd like help with, a little at a time, in an Ask chat with the Chief of Staff. The questions are never "done": nothing blocks, nothing nags, and nothing is saved until the person confirms it. Everything here works with no relay.

Code: `src-tauri/src/services/knowme/` (`mod.rs` service, `cadence.rs`, `proposal.rs`, `sharing.rs`), the `knowMe` kind in `services/inbox/` (`item.rs`, `tool.rs`, `thread.rs`), and `src/components/knowme/`.

## The flow

1. **The to-do.** When onboarding reaches Done (`auth::set_onboarding_step`), `knowme::start` opens the first to-do: "Tell me about your work, and anything at home you'd like help with". `buildInbox` shows it as kind `knowMe` (one line, quiet, not counted in the badge). Later hides it for a week; Skip for a month.
2. **The chat.** The to-do opens the one standing Inbox thread `inbox:knowMe:you` (`InboxItemRef::KnowMe`), a Chief of Staff conversation like any Inbox item's. "Tell Nandi about you" in Network › Sharing › What you offer (`TellNandiAboutYou`) opens the same chat any time, and lists the latest confirmed changes.
3. **The questions** are content, not code: the Chief of Staff's `know-me` playbook (`resources/builtin-agents/chief-of-staff/skills/know-me.md`) says what to ask first, which two or three follow-ups are worth asking (only where the answer changes what Nandi does), and that health, family and money come up only after the person raises them. The thread prompt (`inbox::thread`, `KNOW_ME_GUIDANCE`) gives the card's current state as untrusted `<task-data>` (`item::know_me`: name, description, responsibilities, sharing switches, profile text, served capabilities, groups) and points at the playbook.
4. **The proposal.** In that thread only, the Chief of Staff is offered `knowme_propose` (`DecisionTool::Propose`). Like every Inbox decision tool it is forced to wait (`inbox::tool::gate`), and its card isn't an Inbox line.
5. **The card** (`KnowMeProposalCard`, shown by `ApprovalRequestCard` for `knowme_propose`) lists each change, editable: text, work or personal, "On my card" for work, "Private" or one group for personal, removable notes, offered capabilities and suggestions. **Confirm** calls `confirm_know_me_proposal` (`POST /api/know-me/proposals/{approval_id}/confirm`): the edited proposal is checked, written into the approval's arguments (`db::set_approval_args`), and the approval is approved. **Refuse** is the normal reject: nothing is written.
6. **On confirm** the dispatcher runs the call with the approval's arguments (`inbox::tool::confirmed_args`), so the edits count, then `knowme::apply`: NANDI.md through the gateway service (`GatewayService::save_manifest`, which re-registers the card), My notes through `notes::save_note`, the decision through `inbox::audit::record` (kind `knowMe`, item `you`, the chat named), and the to-do closes (`Cadence::answered`).

## The proposal

`KnowMeProposal` (`proposal.rs`, camelCase on the wire):

| Field              | Type                                                                                 | Goes to                                                                    |
| ------------------ | ------------------------------------------------------------------------------------ | -------------------------------------------------------------------------- |
| `summary`          | string                                                                               | the card's title only                                                      |
| `responsibilities` | `[{text, kind: "work" \| "personal", onCard (work, default true), shareWithGroup?}]` | see below                                                                  |
| `description`      | string?                                                                              | NANDI.md `description`                                                     |
| `canHelpWith`      | string?                                                                              | the body's `## What I can help with` (replaced in place, or added)         |
| `cantHelpWith`     | string?                                                                              | the body's `## What I can't help with`                                     |
| `notes`            | `[{text, private}]`                                                                  | `about-me.md` "Notes", or `private/about-me.md` when private               |
| `offer`            | capability names                                                                     | NANDI.md `policy`: out of `hide`; added to `expose` only if `expose` lists |
| `suggest`          | `[{name, why}]`                                                                      | nothing: shown on the card; the person installs from Library               |

Responsibilities:

- **Work, on the card** → NANDI.md `responsibilities:` (normalized). Whether people see them still depends on "Make reachable" (`relay.enabled`), `relay.share_responsibilities` and `discoverable`.
- **Work, off the card** → My notes `about-me.md`, "## Work".
- **Personal** → My notes `private/about-me.md`, "## Personal". Never NANDI.md: `checked` forces `onCard: false`, and `apply_to_manifest` takes only work items on the card.
- **Personal, shared with a group** → also `private/groups/<group-id>.md` (see below).

`KnowMeProposal::checked` is the one place the rules live (the card and the tool both go through it): text trimmed to one line and capped, at most 20 items, a group only on a personal item and only a group I'm in, `offer` only capabilities agents here serve ("suggest an agent or pack instead"), and a proposal that changes nothing is refused. `add_lines` only adds bullets (once, any case) and moves the "Last updated" line; it never removes what the person wrote.

**The decisions file is in the repo**, so its entry has a fixed subject ("Getting to know you") and a summary of counts (`KnowMeProposal::record`): the work items put on the card are named (they're public anyway), personal ones only counted. No personal text goes into `.nandi/decisions/`.

## Sharing a personal item with one group

Owner decision (2026-11-09): personal items are private by default; the person may share one with **one group they choose**.

- The item is written to `private/groups/<group-id>.md` in My notes (and marked "(shared with <group>)" in `private/about-me.md`). Nothing goes to the relay at that point.
- When someone sends this Nandi a request (`ask-me`, directly to the Chief of Staff or through the gateway's hand-off), the Chief of Staff's turn on it runs in `task:<id>`. `knowme::sharing::prompt_for_task` finds the original requester (`tasks.requester_instance` of the task, or of the gateway task a `-gw` hand-off answers) and, for each group copy (`.nandi/groups/`) that lists both the requester and this Nandi, adds that group's shared items to the system prompt in `<group-shares>`. Other requesters get nothing; the playbook and skill keep `my-notes/private/` private otherwise.
- The answer then travels like any answer: sealed, through the relay.

**Gap:** the relay's people search (`find_people`, the directory) can't find someone by a group-shared item. That needs the relay to publish per-group card fields to members only, which is protocol work in `nandi-spec` and `nandi-relay`. Until then, a group member gets the item only by asking.

## Cadence

`cadence.rs`, stored in `state.db`'s `local_settings` under `knowMe.cadence` (`{open, shownAt, nextAt, snoozed, answeredAt}`):

| Event              | Effect                                                             |
| ------------------ | ------------------------------------------------------------------ |
| onboarding Done    | the first to-do, once (`start`)                                    |
| Later              | closed; the same to-do back in 7 days (`snoozed`)                  |
| Skip               | closed; back in 30 days at the soonest                             |
| a confirmed change | closed; back in 90 days, asking "is this still right?" (`recheck`) |
| ignored            | stays, quietly (not in the badge)                                  |

`tick` (on every `get_know_me`) reopens a closed to-do whose time came, but never a new one sooner than 30 days after the last (`shownAt`). `get_know_me` also starts the first one when onboarding is already Done.

**Why `local_settings`, not the gold copy:** it's when a reminder last showed on this computer, runtime state like `onboarding.step`. It defines nothing, and losing it costs at most one early to-do. The questions are content (the playbook) and the answers are the person's files (NANDI.md, My notes), so nothing here needs a projection or a rebuild.

## Without a model

The to-do stays; the chat needs a model to answer. The built-in help guide has a `know-me` topic (`services/help/guide.json`, `help.topics.know-me.*`) that explains it and links to Network › Sharing and Models & runtimes.

## Commands and events

| Command                    | HTTP                                                | Does                                    |
| -------------------------- | --------------------------------------------------- | --------------------------------------- |
| `get_know_me`              | `GET /api/know-me`                                  | `KnowMeStatus`: to-do, groups, recent   |
| `know_me_todo`             | `POST /api/know-me/todo` `{action}`                 | Later or Skip                           |
| `confirm_know_me_proposal` | `POST /api/know-me/proposals/{approval_id}/confirm` | confirms a card with the person's edits |

`knowme://changed` (`KnowMeChanged`) fires when the to-do or the answers change; `useKnowMe` refetches on it.
