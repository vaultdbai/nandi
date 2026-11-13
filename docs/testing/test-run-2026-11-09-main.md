# Live run on main: sharing, groups and Inbox Ask chats (2026-11-09)

**Build:** app main `0bd4321` (plus the fix below), nandi-relay main `aedacdf`, nandi-spec main `7a4212c` (protocol 7). Web UI built (`pnpm run build`) and served by each headless Nandi.
**Setup:** one relay with a secret and `--visibility groups`, three headless Nandis (Ada, Bob, Cy). Harness: `D:/tmp/nandi-main-1109/` (`start.sh`, `s00-tokens.mjs`, `s01-setup.mjs`, `run.mjs <phase>`, `shots.mjs` for screenshots in headless Edge; findings in `findings.log`; screenshots in `shots/`).

**Result:** 22 checks pass, 0 fail, and 14 screenshots, with no console errors.

| Phase       | Check                                                                                                                                                                                             | Result |
| ----------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ |
| vis-before  | With no groups, nobody sees anybody                                                                                                                                                               | PASS   |
| create      | Ada creates "Finance close"; policy v1 verified; Bob lists it but can't open it                                                                                                                   | PASS   |
| invite      | Wrong code refused; Bob joins with a one-use code (verified, Ask first); Cy can't reuse it; Bob's copy written; Ada sees Bob, Cy sees nobody                                                      | PASS   |
| enforce     | Ada → Bob `ask-me` held for Bob; `pay-invoice` refused by the group policy; Ada → Cy: "You don't share a group with them on this relay"                                                           | PASS   |
| decide      | Bob declines with a reason; Ada sees "The owner declined this request: Not before the board meeting on Thursday"; Bob's `.nandi/decisions/2026-11.md` has the entry (who, item, declined, reason) | PASS   |
| access-none | Bob's access None refuses Ada's question                                                                                                                                                          | PASS   |
| policy2     | v2 reaches Bob verified; Bob (not admin) can't change it                                                                                                                                          | PASS   |
| sharing-off | Bob turns sharing off and leaves Ada's directory; back on, he's back                                                                                                                              | PASS   |
| leave       | Bob leaves; Ada no longer sees him; Bob's copy removed                                                                                                                                            | PASS   |

## Fix from this run

- **An Inbox item's chat never loaded its messages** (a spinner, and a 400 from `GET /api/conversations/{id}`, now `POST /api/rpc/get_conversation`). `snapshots::get_conversation` served only Studio conversations, while Inbox threads belong to the Chief of Staff; unit tests passed because they mock the client. It now also serves Chief of Staff threads (`inbox:` and `intake:`), and nothing else of the Chief of Staff's (`snapshots::readable`; test `studio_and_chief_of_staff_threads_are_readable_other_chats_are_not`).

## Screenshots (shots/)

01 Ada's Home · 02 People on a groups relay (the note) · 03 What you offer: Availability · 04 the offer editor with "Show my responsibilities on my card" · 05 Groups list · 06 group detail: access level and verified policy · 07 members and invites · 08 Create group with the policy template · 09 Bob's Inbox, one line · 10 the item's Ask chat · 11–12 Bob's group detail and access level · 13 Cy: an invite for her · 14 Cy's People: nobody.

## Not covered

- Chat replies from the Chief of Staff: no model is signed in on these Nandis, so the chat shows the item and the input but no reply.
- A personal invite shows the invitee's device ID to the inviter when they can't see each other yet (13 and 07). Small; a follow-up.
