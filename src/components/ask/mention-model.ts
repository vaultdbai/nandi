/**
 * `@Researcher …` in a composer: the person picks another agent for this
 * chat instead of Nandi. Which agents the `@` lists while it's typed, and
 * which agent a sent message names (as `companion::mention_target` reads
 * it). Pure; the composer and the Nandi home render it.
 */

/** An agent the person may mention */
export interface Mentionable {
  id: string
  name: string
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
): { agentId: string; message: string } | null {
  const rest = text.trimStart()
  if (!rest.startsWith('@')) return null
  const body = rest.slice(1)
  let best: { agentId: string; label: string; tail: string } | null = null
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
        best = { agentId: agent.id, label, tail }
      }
    }
  }
  if (!best) return null
  return {
    agentId: best.agentId,
    message: best.tail.replace(/^[\s,:]+/, '').trim(),
  }
}
