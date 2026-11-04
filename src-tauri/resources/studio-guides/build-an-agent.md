---
id: build-an-agent
title: Build an agent
when: Proposing or changing an agent.
---

An agent is a folder in the user's repo: a `skill.md` (frontmatter, then the procedure in Markdown) and an optional `instructions.md` next to it.

## Steps

1. **Ask what it's for, if it isn't clear.** One job per agent. "Summarise my inbox" is an agent; "summarise, then post to Slack, then file a ticket" may be a workflow (read the agent-or-workflow guide).
2. **Check what exists.** Call `nandi_list_agents` first. If an agent already does most of it, propose a change to that one, or a fork, instead of a new one. The official pack (Library › Browse › Nandi Dev Pack) has three:
   - **Chief of Staff** finds who is responsible, asks and assigns work, tracks it, and answers for the user.
   - **PR Reviewer** reviews a GitHub pull request against the user's rules and posts the review after approval.
   - **AWS Ops Analyst** investigates failed AWS Batch jobs and CloudWatch alarms and proposes fixes.
3. **Pick the tools.** Give only the tools the job needs, by ID, from the tool list in your prompt. Fewer tools is safer and gives better results. Call `nandi_list_tools` to see which are switched on, including connectors and pilots. For an outside service or a folder on the user's computer, the agent lists a connector under `connectors`: check `nandi_list_connectors`, and install a missing one with `nandi_save_artifact` (kind `connector`) before saving the agent.
4. **Pick the approvals.** Any tool that changes something (runs commands, writes files, sends messages, posts reviews) belongs in `requires_approval` at first. Tell the user they can relax it once they trust the agent.
5. **Split the text.** The `skill.md` body holds the procedure and the output format: what to do, in which order, what the answer looks like. `instructions.md` holds the user's own rules and policy ("never touch prod", "tests for every bug fix"). Mark it clearly as theirs to edit.
6. **Pick the model.** Leave `model` out, or use the default model from your prompt, unless the user asks for another. Use only models your prompt says this Nandi can call. Call `nandi_list_models` if unsure.
7. **Folders.** Folders on the user's computer come from the filesystem connector's list of shared folders (`nandi_list_connectors` shows each folder's short name and whether it's read only). Give the agent only the ones it needs, by short name: `connectors: [{id: filesystem, folders: [docs, {id: reports, access: read}]}]`. An agent can be narrowed to read only, never given write on a read-only folder. If the folder isn't on the list, tell the user to add it in Library › Connectors › Filesystem; don't invent one.
8. **Network.** Agents have no internet by default. If the job needs a host, add it to `network` and say why. Outside services (GitHub, AWS, an MCP server) come through connectors instead: read the connect-a-service guide.
9. **Long commands.** One `shell` call stops after 30 seconds. For clones, builds or test runs, set `shell_timeout_s` (up to 3600).
10. **Check, then show.** Call `nandi_check_artifact` with the full file and fix every error. Show the file once, with its path, then save it with `nandi_save_artifact` (the user approves the save).

## Agents from the marketplace

- An installed agent runs only after the user clicks **Review and allow** on its page and ticks each kind of access it asks for.
- Its **rules** (instructions.md) are the user's to change: save kind instructions with the whole new text. They're kept as the user's own (Settings › Your rules can reset them), the agent stays installed, and updates keep them. Prefer this to forking when only the rules change.
- Its procedure, tools and model are read-only. To change those, fork it (the Fork button on its page), then change the fork.
- Updates: Library › the pack › ⋯ › **Check for an update**. A fork doesn't get updates.

## Judgement

- Write the procedure as numbered steps the model can follow, and say what to do when a step fails.
- Say what the agent must not do, in plain words.
- Don't invent keys or tool IDs. If something isn't in your prompt, it doesn't exist.
