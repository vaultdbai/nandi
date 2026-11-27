/**
 * `@Researcher …` in a composer: the person picks another agent for this
 * chat instead of Nandi. Which agents the `@` lists while it's typed, and
 * which agent a sent message names (as `companion::mention_target` reads
 * it). Pure; the composer and the Nandi home render it.
 */

/** Who an `@` names: one of the person's agents (the default), a person
 * on their relay, or a group they're in */
export type MentionKind = 'agent' | 'person' | 'group'

/** An agent, person or group the person may mention */
export interface Mentionable {
  id: string
  name: string
  kind?: MentionKind
  /** A person's presence, when the relay says */
  online?: boolean
}

/**
 * Everyone an `@` may name: the person's agents, then, while the relay is
 * connected, the people on it (not the person) and the groups they're in.
 * No relay: agents only.
 */
export function mentionables(
  agents: readonly { id: string; name: string }[],
  people: readonly {
    instanceId: string
    displayName: string
    online: boolean
  }[],
  groups: readonly { id: string; name: string }[],
  me: string | null
): Mentionable[] {
  return [
    ...agents.map(agent => ({
      id: agent.id,
      name: agent.name,
      kind: 'agent' as const,
    })),
    ...people
      .filter(person => person.instanceId !== me && person.displayName.trim())
      .map(person => ({
        id: person.instanceId,
        name: person.displayName.trim(),
        kind: 'person' as const,
        online: person.online,
      })),
    ...groups.map(group => ({
      id: group.id,
      name: group.name,
      kind: 'group' as const,
    })),
  ]
}

/** Agents listed under the composer while an `@` is typed */
export const MAX_MENTIONS = 6

/**
 * While the draft is a lone `@word` (nothing after it yet), the agents
 * whose name or ID starts with the word
 */
export function mentionOptions(
  draft: string,
  agents: readonly Mentionable[]
): Mentionable[] {
  const match = /^@(\S*)$/.exec(draft.trimStart())
  if (!match) return []
  const query = (match[1] ?? '').toLowerCase()
  return agents
    .filter(
      agent =>
        agent.name.toLowerCase().startsWith(query) ||
        agent.id.toLowerCase().startsWith(query)
    )
    .slice(0, MAX_MENTIONS)
}

/** The draft once an agent is picked: its name, then the message */
export function withMention(agent: Mentionable): string {
  return `@${agent.name.trim() || agent.id} `
}

/**
 * The agent a message starting `@Name` is for, and the message without
 * it; the longest name wins, and the name must end at a space, `,` or `:`
 */
export function mentionTarget(
  text: string,
  agents: readonly Mentionable[]
): { id: string; kind: MentionKind; message: string } | null {
  const rest = text.trimStart()
  if (!rest.startsWith('@')) return null
  const body = rest.slice(1)
  let best: {
    id: string
    kind: MentionKind
    label: string
    tail: string
  } | null = null
  for (const agent of agents) {
    for (const label of [agent.name.trim(), agent.id]) {
      if (!label) continue
      const head = body.slice(0, label.length)
      const tail = body.slice(label.length)
      const boundary = tail === '' || /^[\s,:]/.test(tail)
      if (
        boundary &&
        head.toLowerCase() === label.toLowerCase() &&
        (!best || label.length > best.label.length)
      ) {
        best = { id: agent.id, kind: agent.kind ?? 'agent', label, tail }
      }
    }
  }
  if (!best) return null
  return {
    id: best.id,
    kind: best.kind,
    message: best.tail.replace(/^[\s,:]+/, '').trim(),
  }
}
