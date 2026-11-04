---
id: write-a-workflow
title: Write a workflow
when: Proposing or changing a workflow.
---

A workflow is a YAML file in the user's repo. Its file name matches its `id`. Unknown keys are errors, so typos fail loudly.

## Shape

- A workflow has an `id`, a `name`, an optional `description`, and a list of `steps`.
- Each step has its own `id` and usually names an `agent` and the `input` message it gets.
- A step reads an earlier step's final reply with `{{<step id>.output}}`. That's the only way data moves between steps.
- A step can **ask the user first**: the run pauses before its message is sent, and the user approves or rejects it in the Inbox or on the workflow's page.
- A step can be **critical**: it goes ahead of other background work in its agent's queue.
- A step can be **manual**: no agent, just instructions for a person and an optional deadline. The run waits until the person completes it in Work, and their reply becomes that step's output.
- A step can **ask a colleague**: an `ask:` block with exactly one of `find: <topic>` (whoever owns it on the network; the user approves who before it's sent), `person: <instance id>` or `capability: <id>`, plus `input:` (what to ask) and an optional `due:` (`YYYY-MM-DD` or `N days`). The run waits, days if needed, and the answer becomes that step's output. No `agent:` on an ask step.

## Steps

1. Call `nandi_list_workflows` and `nandi_list_agents`. Every step's agent must exist, or be proposed in the same answer.
2. Agree the steps with the user in plain words: what each does, which agent, what it passes on.
3. Put the approval on the step that changes something outside (sends, posts, writes), not on the reading steps.
4. Keep inputs short and specific. Tell each agent what it gets and what to return, so the next step can use it.
5. Call `nandi_check_artifact` with kind `workflow` and the full YAML. Fix every error and the warnings you can. It catches missing agents and unknown keys.
6. Show the file once, with its path, then save it with `nandi_save_artifact` (the user approves the save).
7. If it should run on a schedule or an event, read the set-up-a-trigger guide.

## Judgement

- Three to five steps is typical. More usually means two workflows.
- If one step's agent would just echo its input, drop the step.
- A failed step stops the run. Say in the step's input what the agent should do if it can't finish.
