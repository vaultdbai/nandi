/**
 * A `connector_access` approval in plain words: who wants what, which
 * answers it offers, and what was chosen. Pure: the access card, the Inbox
 * summary and their tests read from these (`services/access.rs`
 * `AccessRequest` is the args' shape).
 */

import type { AccessChoice, GrantAccess } from '@/lib/bindings'

/** The approval tool a call without a grant asks through */
export const ACCESS_TOOL = 'connector_access'

export type AccessFamily = 'email' | 'calendar' | 'files' | 'connector'

export interface AccessConnector {
  id: string
  name: string
}

/** The args of a `connector_access` approval */
export interface AccessRequest {
  agent: string
  family: AccessFamily
  connectors: AccessConnector[]
  access: GrantAccess
  tool: string
  details: string[]
  askEveryTime: boolean
  choice: AccessChoice | null
  decidedBy: string | null
}

const FAMILIES: readonly AccessFamily[] = [
  'email',
  'calendar',
  'files',
  'connector',
]
const CHOICES: readonly AccessChoice[] = [
  'allowOnce',
  'allowLocal',
  'allowSaved',
  'deny',
]

function text(value: unknown): string {
  return typeof value === 'string' ? value.trim() : ''
}

/** The args as an access request, or null when they aren't one */
export function parseAccessRequest(args: unknown): AccessRequest | null {
  if (typeof args !== 'object' || args === null) return null
  const raw = args as Record<string, unknown>
  const family = FAMILIES.find(f => f === raw.family)
  if (!family) return null
  const connectors = Array.isArray(raw.connectors)
    ? raw.connectors.flatMap((c: unknown) => {
        if (typeof c !== 'object' || c === null) return []
        const entry = c as Record<string, unknown>
        const id = text(entry.id)
        if (!id) return []
        return [{ id, name: text(entry.name) || id }]
      })
    : []
  const choice = CHOICES.find(c => c === raw.choice) ?? null
  return {
    agent: text(raw.agent),
    family,
    connectors,
    access: raw.access === 'write' ? 'write' : 'read',
    tool: text(raw.tool),
    details: Array.isArray(raw.details)
      ? raw.details.map(text).filter(Boolean)
      : [],
    askEveryTime: raw.askEveryTime === true,
    choice,
    decidedBy: text(raw.decidedBy) || null,
  }
}

/** An i18n key under `access.sentence.` and its values */
export interface AccessSentence {
  key:
    | 'emailRead'
    | 'emailWrite'
    | 'calendarRead'
    | 'calendarWrite'
    | 'filesRead'
    | 'filesWrite'
    | 'connectorRead'
    | 'connectorWrite'
  values: { agent: string; names: string }
}

/** "<Agent> wants to read your email (Outlook)" and the like */
export function accessSentence(
  request: AccessRequest,
  agentName: string
): AccessSentence {
  const access = request.access === 'write' ? 'Write' : 'Read'
  return {
    key: `${request.family}${access}`,
    values: {
      agent: agentName,
      names: request.connectors.map(c => c.name).join(', '),
    },
  }
}

/** The answers offered: a policy that asks every time allows only once */
export function accessChoices(request: AccessRequest): AccessChoice[] {
  return request.askEveryTime
    ? ['allowOnce', 'deny']
    : ['allowOnce', 'allowSaved', 'allowLocal', 'deny']
}

/** "Allow for this agent" keeps the grant saved (backed up) by default */
export const DEFAULT_AGENT_SCOPE: Extract<
  AccessChoice,
  'allowSaved' | 'allowLocal'
> = 'allowSaved'

/**
 * What was decided, as a key under `access.decided.`: the choice in the
 * args when Rust wrote it, else what the approval's status says
 */
export function decidedKey(
  request: AccessRequest,
  status: 'approved' | 'rejected' | 'expired'
): AccessChoice | 'allowed' | 'expired' {
  if (request.choice) return request.choice
  if (status === 'expired') return 'expired'
  return status === 'approved' ? 'allowed' : 'deny'
}

/** `pr-reviewer` → "PR Reviewer" (short words are acronyms) */
export function agentLabel(id: string): string {
  return id
    .split(/[-_]/)
    .filter(Boolean)
    .map(word =>
      word.length <= 2
        ? word.toUpperCase()
        : `${word[0]?.toUpperCase() ?? ''}${word.slice(1)}`
    )
    .join(' ')
}
