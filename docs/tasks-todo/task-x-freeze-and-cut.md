# Task: Freeze or cut features that serve none of trust, state, verification

**From:** feature review against "trust, state, verification" (2026-11-08)
**Serves:** focus
**Size:** S per group. **The owner decides each group** before anything is removed.

"Freeze" means no new work and no fixes beyond breakage. "Cut" means remove the code, its strings and its catalog entries (no shims: nothing is released before them).

| Group                   | IDs                               | Why                                                                         | Proposed | Owner |
| ----------------------- | --------------------------------- | --------------------------------------------------------------------------- | -------- | ----- |
| Companion and mascot    | AI-11..18, SH-33, SH-34, ST-29    | UI polish; serves none of the three                                         | Freeze   |       |
| Git-URL teams           | TM-02..08                         | A second trust model next to relay groups; TM-05 Partial with no UI         | Cut      |       |
| Capability admin UI     | CP-12..15, CP-17, CP-20           | Heavy UI over a model-shaped problem; CP-20 a known issue                   | Freeze   |       |
| Visual workflow editors | WF-10, WF-11, WF-12, WF-13, WF-16 | Workflows are YAML the model writes (WF-06); keep the engine and the record | Freeze   |       |
| Half-built runtimes     | RT-12, RT-23, RT-28               | Stubs with no way in (RT-23: no command to register; RT-28: backend only)   | Cut      |       |
| Bedrock extras          | MD-20, MD-21                      | Partial; LiteLLM gateway unused                                             | Freeze   |       |
| PR step                 | AI-06 `remoteBranchAndPr`         | Says it opens a PR and doesn't                                              | Cut      |       |
| Gateway federation      | GW-17                             | Multi-hop trust before single-hop is hardened                               | Freeze   |       |
| Publisher tools         | LB-17, CP-31                      | Publishing is the owner's own flow for now                                  | Freeze   |       |
| Theme extras            | ST-28                             | Palettes and icon choice                                                    | Freeze   |       |

Related, already in tasks: `task-x-companion-bison.md` (freeze with the companion group).

## Done when

Each row has an owner decision, cut rows are removed with their catalog entries, and frozen rows say "Frozen" in `docs/testing/`.
