---
id: network-and-delegation
title: Network and delegation
when: Who is responsible, asking or assigning work, sharing, trust.
---

People connect their Nandis through a relay. Once connected, they can send each other work, and their agents can call each other's agents.

## Who is responsible for what

Each person lists what they're responsible for in Network › Sharing › What you offer (`NANDI.md`, under responsibilities). Their Nandi shares that with everyone on the relay, even when they keep the rest of their profile private. Encourage the user to fill theirs in: it's how others find them.

An agent finds people with `find_people` (a topic such as "SOX audit", or empty for everyone). It searches live what each person published: responsibilities, profile, and their agents and what each serves. Results say what matched, online or last seen, and how to ask: the person's gateway target, or a matching agent's target and capability. `my_requests` lists requests both ways (asked by the user, asked of the user), with what waits on the user first. Both tools are offered only to agents that list them in `allowed-tools`.

The official **Chief of Staff** agent does all of this for the user: it finds owners, asks people and their agents, assigns work, tracks it, and answers for the user. Suggest it before building something similar.

## Asking someone

- To ask one person or one agent directly, use `delegate_to_agent`. The user approves each call first.
- A question for a person goes to their gateway with capability ask-me, which their Chief of Staff answers. Work for a person uses assign-me, which waits for their approval by default. It lasts until they mark it done or decline it in their Inbox (due in 7 days unless the input has due as YYYY-MM-DD); delegate_to_agent returns status assigned at once, and my_requests shows it until it's done. A task for a specific agent (a teammate's PR Reviewer serving review-pr) goes to that agent's target with a capability it serves.
- A request waits up to 10 minutes for an answer. Work that takes days isn't tracked through to done yet: say so.
- Call `nandi_network_snapshot` first: who is online and who takes requests. Don't send work to someone offline without saying it will wait.
- The owner's trust policy decides whether the task runs on their side. A request straight to one of their agents needs them to mark the user as a **trusted peer**, or it becomes a trust request in their Inbox. Questions through their gateway don't need that.
- For bigger asks with several steps, start a Request instead (read the requests-and-work guide).

## Capabilities

A capability is a named thing agents can do for others, like summarising a paper. Call `nandi_list_capabilities` to see which exist, which an agent here provides, and how many providers are online. When an agent delegates by capability, Nandi picks a provider: this computer first, then the least busy.

## Sharing an agent

For the user's agent to take work from others:

1. Set `accepts_delegation` to true, and list what it offers with `serves` (or `provides` for capability IDs).
2. Choose who may call it: allow, ask, or deny per caller, in Network › Sharing.
3. The user must be connected to a relay and reachable. If Network shows "Your Nandi is not visible to the network", **Make Reachable** fixes it. Network › Connection has a checklist with one fix per missing item.

Propose the skill.md change, run `nandi_check_artifact`, show it, then save it with `nandi_save_artifact`.

## Trust

Each member has three switches: trusted peer, pause delegations, and block. Trusted peers are recorded in the user's `NANDI.md`.

## Judgement

- The relay today is local-network only, open and unencrypted. Say so before the user shares anything sensitive.
- Never invent members or capabilities. Use only what the tools return.
