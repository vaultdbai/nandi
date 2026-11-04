---
id: agent-or-workflow
title: One agent or a workflow
when: One agent or a workflow of steps.
---

A workflow chains agents. Each step sends a message to one agent and waits for its final reply before the next step starts. Data passes between steps only through earlier replies (`{{step.output}}`); agents can't share files.

## Use one agent when

- One model with one set of tools can do the whole job.
- The parts are just steps of a procedure ("read the diff, check the rules, report"). Put them in the agent's instructions.
- The user wants to chat back and forth with it.

## Use a workflow when

- Different parts need **different agents**: different tools, models, or rules. For example research, then summarise, then publish.
- A **person must approve** before a step happens ("draft, then I approve, then send"). A workflow step can ask first.
- A **person must do something** in the middle (sign off, upload, tag a laptop). A workflow can have a manual step that waits for days.
- The same chain should run **on a schedule or an event**. A trigger can start a workflow.
- One part is risky and the rest isn't. Isolate the risky part in its own agent with tight tools and approvals.

## Steps

1. Call `nandi_list_agents` and `nandi_list_workflows`. Reuse what exists.
2. If one agent is enough, read the build-an-agent guide.
3. If it's a workflow, read the write-a-workflow guide. List the steps for the user in plain words first and confirm them, then write the file.
4. If the user only wants it done once, now, don't build anything: start a Request (read the requests-and-work guide).

When in doubt, start with one agent. It's easier to split a working agent into a workflow later than to debug a workflow of agents that don't work yet.
