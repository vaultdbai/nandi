/**
 * What you offer (network.md §4.4): each capability a local agent serves,
 * in one of three columns from the gateway policy, and the policy an edit
 * of those columns saves. Capabilities in the policy that no local agent
 * serves are kept as they are.
 */
import type { GatewayPolicy } from '@/lib/bindings'

export type OfferColumn = 'offered' | 'approval' | 'hidden'

export const OFFER_COLUMNS: OfferColumn[] = ['offered', 'approval', 'hidden']

export interface ServedBy {
  /** The agent's display name */
  agent: string
  capabilities: string[]
}

export interface OfferedCapability {
  name: string
  /** Who serves it, sorted */
  agents: string[]
  column: OfferColumn
}

/** Hidden wins, then "only what's exposed", then approval */
export function capabilityColumn(
  name: string,
  policy: GatewayPolicy
): OfferColumn {
  if (policy.hide.includes(name)) return 'hidden'
  if (policy.expose.length > 0 && !policy.expose.includes(name)) {
    return 'hidden'
  }
  return policy.requireApproval.includes(name) ? 'approval' : 'offered'
}

export function buildOffer(
  served: ServedBy[],
  policy: GatewayPolicy
): OfferedCapability[] {
  const agents = new Map<string, Set<string>>()
  for (const { agent, capabilities } of served) {
    for (const name of capabilities) {
      const set = agents.get(name) ?? new Set<string>()
      set.add(agent)
      agents.set(name, set)
    }
  }
  return [...agents.entries()]
    .map(([name, by]) => ({
      name,
      agents: [...by].sort((a, b) => a.localeCompare(b)),
      column: capabilityColumn(name, policy),
    }))
    .sort((a, b) => a.name.localeCompare(b.name))
}

const unique = (names: string[]) => [...new Set(names)]

/**
 * The policy for `choices` (capability → column). An empty `expose`
 * ("everything is offered") stays empty, hiding through `hide`.
 */
export function applyOffer(
  policy: GatewayPolicy,
  choices: Record<string, OfferColumn>
): GatewayPolicy {
  const local = (name: string) => name in choices
  const chosen = (column: OfferColumn) =>
    Object.keys(choices).filter(name => choices[name] === column)
  const visible = [...chosen('offered'), ...chosen('approval')]
  return {
    ...policy,
    hide: unique([...policy.hide.filter(n => !local(n)), ...chosen('hidden')]),
    requireApproval: unique([
      ...policy.requireApproval.filter(n => !local(n)),
      ...chosen('approval'),
    ]),
    expose:
      policy.expose.length === 0
        ? []
        : unique([...policy.expose.filter(n => !local(n)), ...visible]),
  }
}

/** The capabilities whose column `choices` changes, for the save summary */
export function offerChanges(
  before: OfferedCapability[],
  choices: Record<string, OfferColumn>
): { name: string; column: OfferColumn }[] {
  return before
    .filter(c => choices[c.name] !== undefined && choices[c.name] !== c.column)
    .map(c => ({ name: c.name, column: choices[c.name] as OfferColumn }))
}

/** Longest responsibility kept (as NANDI.md's `responsibilities:`) */
export const MAX_RESPONSIBILITY_CHARS = 100
/** Most responsibilities kept */
export const MAX_RESPONSIBILITIES = 50

/**
 * The editor's one-per-line text as `responsibilities:`: trimmed, blank
 * lines dropped, each cut to {@link MAX_RESPONSIBILITY_CHARS}, repeats
 * (ignoring case) dropped, at most {@link MAX_RESPONSIBILITIES}. The same
 * rules the backend applies when it writes NANDI.md.
 */
export function parseResponsibilities(text: string): string[] {
  const seen = new Set<string>()
  const kept: string[] = []
  for (const line of text.split(/\r?\n/)) {
    const item = Array.from(line.trim())
      .slice(0, MAX_RESPONSIBILITY_CHARS)
      .join('')
      .trimEnd()
    const key = item.toLowerCase()
    if (!item || seen.has(key)) continue
    seen.add(key)
    kept.push(item)
    if (kept.length === MAX_RESPONSIBILITIES) break
  }
  return kept
}

/**
 * What others read about this Nandi: the one-line description, then the
 * NANDI.md body without its `<!-- -->` comments, which stay private (the
 * backend's `public_body` strips them before the body is shared).
 */
export function publicAbout(description: string, body: string): string {
  const visible = body
    .replace(/<!--[\s\S]*?(?:-->|$)/g, '')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
  return [description.trim(), visible].filter(Boolean).join('\n\n')
}
