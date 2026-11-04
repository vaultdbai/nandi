# Task: Agents in the left menu, grouped into a folder per pack

**From:** owner 2026-10-30: "people will have lots of agents"; in the left menu, Chief of Staff on top with no folder, and agents installed through packs inside a folder named after the pack.
**Size:** S, mostly frontend. After v1 (demo scope rule).
**Related:** Chief of Staff built into Nandi (`services/builtin_agents.rs`, done 2026-10-30), `task-x-editable-packs.md`.

## Today

- `src/components/layout/nav-tree.ts` `agentNodes`: built-in agents (Chief of Staff) first and always shown, then the rest flat, sorted by status (waiting, running, error, idle) then name. After `LIST_LIMIT = 8` comes "All agents (n)".
- The file's rule is "Two levels, never more".
- Each agent already has `origin`: `local`, `publisher {publisher}` or `pack {pack}` (the pack ID, not its display name).
- `AgentsListView.tsx` (All agents) is ungrouped too (`sortAgents`).

## Decisions (owner, 2026-10-30)

- Chief of Staff stays on top, with no folder.
- Agents the user made stay flat, right under Chief of Staff.
- Agents installed with a pack go in a folder named after the pack.
- Agents installed on their own from the marketplace go in a folder per publisher.

```
Agents
  Chief of Staff
  My Release Bot
  ▸ Nandi Dev Pack (2) ●
      PR Reviewer
      AWS Ops Analyst
  ▸ acme (1)
```

## Work

- [ ] `nav-tree.ts`: built-in first, local flat, then one folder node per pack and per publisher, sorted by name, with status-then-name order inside. Change the "two levels" rule to "Agents gets pack folders, nowhere else" (comment and `docs/developer/` sidebar notes).
- [ ] Folder label: the pack's display name. Add `name` to `AgentOrigin::Pack` (read from the pack's `.nandi/manifest.yaml` in `agent_locator`), regenerate bindings.
- [ ] A collapsed folder shows the most urgent dot of its agents (waiting, then running, then error), so an approval isn't hidden. A folder with an agent waiting for approval opens by itself.
- [ ] Remember open and closed folders in the UI store (saved like the other sidebar expansion state).
- [ ] Drop `LIST_LIMIT` for agents (folders replace it); keep "All agents (n)".
- [ ] Group `AgentsListView` the same way.
- [ ] Tests: `nav-tree.test.ts` (grouping, order, folder dot, built-in on top), `Sidebar.test.tsx` (expand and collapse a folder), the All agents page.
- [ ] Update the feature test catalog entry for the sidebar in `docs/testing/`.
