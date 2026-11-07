# Team Brain Live Run (2026-11-05)

The first live multi-Nandi run since sealing (protocol 4) and routing at the relay (protocol 5), and the first of protocol 6 (relay design steps 11 to 16: store-and-forward, failover, the team inbox, reassignment, handover, the management API).

## Setup

| Part   | What                                                                                                                                                                       |
| ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Relay  | `nandi-relay` debug build, `ws://127.0.0.1:18911`, `NANDI_RELAY_SECRET` set, `--data-dir` on disk, default timings (90 s pickup window, 5 min handover grace)              |
| Nandis | Three headless debug builds of branch `team-brain-11-16` (`nandi --headless`), each with its own `NANDI_HOME`: Ada (requester), Bob and Cy (providers)                     |
| Join   | Each instance's token from `GET /relay/token` with the secret, given as `NANDI_RELAY_URL=ws://…/relay?token=…`                                                             |
| Driver | Node scripts over each Nandi's HTTP API and the relay's `/team/*` API. The Chief of Staff answered with the host's Claude sign-in; each person had one note to answer from |

## Results

| #   | Scenario                                                                                                                     | Result                                                                                                                                                                                                          |
| --- | ---------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1   | **Ask a person.** Ada asks Bob (`ask-me` to Bob's gateway); Bob's Chief of Staff answers from his notes                      | Pass. Answer correct, the relay's ledger shows `completed`, kind `ask`, one attempt                                                                                                                             |
| 2   | **Assign work.** Ada assigns Bob; Bob's gateway holds it, Bob approves, then marks it Done                                   | Pass. Ada's history ends `completed` with Bob's note                                                                                                                                                            |
| 3   | **Away-delegate.** Bob's NANDI.md says away, Cy covers; Ada asks Bob by name                                                 | Pass. Cy answered; the thread's reasons: `covers` (Cy for Bob), `free`                                                                                                                                          |
| 4   | **Offline then online.** Cy stopped; Ada asks Cy; Cy started again                                                           | Pass. Ada's row and the relay both `submitted` while Cy was away; Cy got it on start and the answer reached Ada                                                                                                 |
| 5   | **Provider reassignment.** Bob hands Ada's held assignment to Cy                                                             | Pass. Back to Ada refused (400). Same task ID at Cy; Ada's row moved to Cy with "Reassigned to Cy Young by Bob Marley (Cy owns DB credentials)"; done by Cy, Ada saw `completed`                                |
| 6   | **Manager reassignment.** The operator moves Bob's held assignment to Cy's gateway (`POST /team/tasks/{id}/reassign`)        | Pass. 202 "Waiting for Bob Marley's Nandi to reseal it"; Cy got it 2 s later; Ada's thread says "Reassigned to Cy Young by Maya (manager)"                                                                      |
| 7   | **Manager reassignment after the provider restarted** (Bob no longer holds the opened request)                               | Pass. The relay asked Ada, the author, to reseal; Cy got it                                                                                                                                                     |
| 8   | **Management API.** Members, the operator's availability for Bob (away, Cy covers), stats, audit, open tasks, a wrong secret | Pass. A question to Bob went to Cy while the operator's availability held; wrong secret 401; stats per member; reassignments and availability audited                                                           |
| 9   | **Calendar handover.** The same calendar trigger on Bob (primary) and Cy (backup); Bob stopped before it was due             | Pass. About 5 minutes after the time Cy fired as backup with "the primary's Nandi didn't answer"; `GET /team/watches` shows `handed_over`, `no_answer`                                                          |
| 10  | **Relay down.** The relay stopped                                                                                            | Pass. The cached network listing answers; refresh answers 503 `relay_unavailable`; a plain request to Bob fails with `relay_unavailable`; an assignment waits in the outbox (`submitted`); local screens answer |
| 11  | **Relay back.** The relay started again                                                                                      | Pass after fix 2: the outbox sent the assignment and Bob's gateway holds it                                                                                                                                     |

People search with the relay down (strict, decision 2) and failover of a refused request to the next candidate were not driven live; they are covered by `pnpm run test:offline`, `a2a::tests`, `relay::sealed_tests` and nandi-relay `tests/dispatch.rs`.

## Bugs found and fixed

1. **Available again didn't clear NANDI.md's `availability:`.** Writing the manifest kept keys it wasn't given, so once away, a person stayed away. Fixed in `gateway/manifest.rs` (`serialize` drops the old block; test `being_available_again_drops_the_old_availability`). Verified on the rerun: Bob available again, and the relay's directory says so.
2. **The outbox lost a request when the relay came back.** It flushed as soon as the socket opened, before this Nandi and the provider had registered, and the relay refused the provider's gateway because its card wasn't resent yet; the refusal abandoned the request. Fixed on both sides: the relay counts the member's stored gateway card until the live one comes (nandi-relay `dispatch.rs`, test `a_member_back_before_its_gateway_card_still_gets_its_work`); the client waits for its registration before submitting, and the outbox abandons only what the relay refused (`REFUSED_BY_RELAY`), retrying anything that didn't get there. Verified on the rerun.

## Rerun on Merged Main (2026-11-07)

The same scenarios on `main` after the team brain, live UI, pull-only images, connector hub and social pack branches were merged, against nandi-relay main. Harness `D:/tmp/nandi-team-1106`. All eleven pass.

One bug found and fixed: **a provider's reassignment of an assignment skipped the teammate's approval.** The candidate was the teammate's agent card (`cy/chief-of-staff`), so Cy's Chief of Staff ran the assignment as a task instead of Cy's gateway holding it. A request that came in through the gateway now goes to the teammate's gateway (`a2a/reassign.rs` `handed_to`). Verified on the rerun: Cy's Nandi held it, Cy approved and closed it, Ada saw it completed.

Failover after a refusal was tried and not reached: a question with no target goes to Ada's own Chief of Staff first, so no remote refusal happens. It stays covered by nandi-relay `tests/dispatch.rs` and `a2a::tests`. Strict people search with the relay down needs a model run (`find_people` is an agent tool); `pnpm run test:offline` covers it.

## Harness

`D:/tmp/nandi-team-1105` (`start.sh`, `stop.sh`, `s00-tokens.mjs`, `s01-setup.mjs`, `run.mjs <phase>`, `findings.log`). Local only.
