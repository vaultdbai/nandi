# Weekly Retro: Nandi Gets Better From Your Corrections

**From:** owner, 2026-11-23 brainstorm: "lets focus on first part and lets add that self improvement todo in 0.3 and add it on website".
**Serves:** state (every change is a gold copy commit with its reason), trust (it never gives itself more access), verification (the check after each change).
**Order:** for 0.3.0.
**Related:** [task-x-run-knowledge.md](task-x-run-knowledge.md) (lessons from runs), [task-x-run-plans-and-playbooks.md](task-x-run-plans-and-playbooks.md) (plan first, already built).

## The Idea

Nandi's code never changes itself. What gets better is state already in the gold copy: notes, `agent.md` prompts, workflows and triggers. Once a week, the Chief of Staff reads what the person corrected and proposes a few changes. The person says yes or no. Nandi then checks whether the change helped.

The model does the reading and the proposing. Nandi builds the record, the approval, the limits and the check.

## The Loop

| Step    | What happens                                                                                                           | Who     |
| ------- | ---------------------------------------------------------------------------------------------------------------------- | ------- |
| Observe | Last week's runs: rejections, drafts edited before approval, retries, failures                                         | records |
| Reflect | The Chief of Staff reads them on a weekly trigger                                                                      | model   |
| Propose | At most three changes, each an Inbox item: the diff and the runs that prompted it                                      | model   |
| Apply   | On yes, one gold copy commit with the reason and the run links. Undo in one click                                      | Nandi   |
| Verify  | Compare the agent's edit and rejection rate over its next runs with the runs before. No better: say so, offer the undo | Nandi   |

## Rules

| ✅ A proposal may                                     | ❌ A proposal may never                                                 |
| ----------------------------------------------------- | ----------------------------------------------------------------------- |
| Edit an `agent.md`, a note, a workflow or a trigger   | Add a tool, a site to `network.allow`, a secret, a connector or a pilot |
| Narrow a grant or an allowed site                     | Remove an approval or touch the approval policy or the sandbox          |
| Cite the runs it learned from                         | Turn text from an email or a web page into an instruction               |
| Suggest an agent take on an action you always approve | Make that action automatic without the person's yes                     |

- The ❌ column is checked in code when a proposal is applied, not only in the prompt.
- Signed pack agents are never edited. The retro may propose a local copy instead.
- The retro runs within a token budget and never more than once a week per person.

## Slices

1. **The retro itself.** The Chief of Staff's `agent.md` and a weekly trigger. It reads last week's run and approval records and sends up to three Inbox proposals. First check what the Chief of Staff can already read; add a read-only tool only if something is missing.
2. **Apply and undo.** A yes writes one gold copy commit with the reason and the run links. Undo reverts it. The ❌ check refuses a proposal that widens anything.
3. **The check.** Edit and rejection rate per agent, before and after. One calm Inbox note when a change didn't help.

## Tests

- A proposal that adds a tool, a site or a secret is refused at apply time.
- A proposal against a signed pack agent is refused.
- Apply then undo leaves the gold copy as it was.
- The before and after rate is computed from run records alone.

## Status (2026-11-23)

The Chief of Staff is now Nandi, the default agent. Its `agent.md` is at the word limit, so the retro's instructions are the playbook `skills/weekly-retro.md` and the trigger `triggers/weekly-retro.yaml` (Fridays at 17:00). Nandi could read no run or approval records, so the retro got three host tools, offered only in that trigger's run: `retro_runs` and `retro_read` (read-only) and `retro_propose` (records a proposal, changes nothing). Developer notes: [triggers.md](../developer/triggers.md#the-weekly-retro). Test catalog: AG-40.

- [x] Slice 1: the playbook, the weekly trigger (at most once in six days, `RetroGuard`), the week's records within a budget, at most three proposals a week, each an Inbox item with its diff and its runs
- [x] Slice 2: Apply is one gold copy commit with the reason and the run IDs; Undo is one more commit. The ❌ column is checked in code at propose and at apply (`domain/retro.rs`); signed pack agents are refused
- [x] Slice 3: corrections per decision, four weeks before against since, from `state.db` alone; one note with Undo and Keep it when a change didn't help
- [x] Tests: widening refused at apply, signed pack refused, apply then undo leaves the gold copy as it was, rates from run records alone, the Inbox item's shape
- [ ] Notes in `my-notes/` aren't a target: they aren't in the gold copy, and Nandi already writes them itself
- [ ] "Suggest an agent take on an action you always approve" is only a line in the summary; there is no proposal kind for it
- [ ] Commit messages name run IDs, not links: the app has no run deep link yet

## Not in This Task

- Reviewing all agents together (merge, retire, split). Later, once there is enough history.
- Replaying past runs on a new model to drop old workarounds. Later.
- Plan-first runs: already built, see [task-x-run-plans-and-playbooks.md](task-x-run-plans-and-playbooks.md). The retro does not change them.

## Open

- Can one proposal span agents ("you prefer short replies" for all of them)? More useful, harder to verify. Start with one agent per proposal.
