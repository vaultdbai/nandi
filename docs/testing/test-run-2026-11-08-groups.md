# Live run: groups on a public-style relay (2026-11-08)

**Builds:** app `feat/groups` (f3bba41, plus the fix below), nandi-relay `feat/groups` (e869622), nandi-spec `feat/groups` (75b9f8a, protocol 7).
**Setup:** one relay with a secret and `--visibility groups`, and three headless Nandis (Ada, Bob, Cy) on it, each with sharing on ("Make reachable") and responsibilities. Harness: `D:/tmp/nandi-groups-1108/` (`start.sh`, `s00-tokens.mjs`, `s01-setup.mjs`, `run.mjs <phase>`; findings in `findings.log`).

| Phase       | Check                                                                                                                  | Result                         |
| ----------- | ---------------------------------------------------------------------------------------------------------------------- | ------------------------------ |
| vis-before  | With no groups, nobody sees anybody (directory)                                                                        | PASS                           |
| create      | Ada creates "Finance close"; her policy v1 is verified; she is admin                                                   | PASS                           |
| create      | Bob lists the group (name, description) as a non-member                                                                | PASS                           |
| create      | Bob can't open it: "Only the group's members can do that"                                                              | PASS                           |
| invite      | A wrong code is refused; Bob joins with Ada's one-use code; the policy is verified on Bob; access starts as Ask first  | PASS                           |
| invite      | Cy can't reuse the one-use code                                                                                        | PASS                           |
| invite      | Bob's copy `.nandi/groups/<id>.md` is written                                                                          | PASS                           |
| invite      | Ada now sees Bob; Cy still sees nobody                                                                                 | PASS                           |
| enforce     | Ada → Bob `ask-me` is held for Bob's approval (Ask first, `approval: always`)                                          | PASS                           |
| enforce     | Ada → Bob `pay-invoice` is refused: "Not accepted here. Finance close: the group's policy doesn't allow 'pay-invoice'" | PASS                           |
| enforce     | Ada → Cy (no shared group) doesn't go                                                                                  | PASS after the fix (see below) |
| access-none | Bob sets his access to None; Ada's question is refused: "the owner takes no requests through this group"               | PASS                           |
| policy2     | Ada sets policy v2; Bob gets it, verified, and his copy says version 2                                                 | PASS                           |
| policy2     | Bob, not an admin, can't change the policy                                                                             | PASS                           |
| sharing-off | Bob turns sharing off and leaves Ada's directory; turning it on brings him back                                        | PASS                           |
| leave       | Bob leaves; Ada no longer sees him; Bob's copy is removed                                                              | PASS                           |

## Fix from this run

- **Ada → Cy said "Cy hasn't published keys… they need to update Nandi".** A groups relay hides the keys of people you share no group with, and the client read the missing keys as an outdated Nandi. Now, on a groups relay, missing keys mean `no_shared_group`: "You don't share a group with them on this relay" (`relay/client.rs` `keys_for`). Test: `relay::tests::on_a_groups_relay_someone_outside_our_groups_is_named_as_such`.

## Not covered live

- Model answers: the held question was not approved and answered, because that needs a model sign-in.
- "Fully automated" running without approval, `daily_limit`, `hours` and `first-time`: covered by unit tests (`groups::enforce`) only.
- The Groups screen itself: checked with Vitest (`GroupsView.test.tsx`), not by clicking through.
