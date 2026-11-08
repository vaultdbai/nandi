/**
 * A `connector_access` approval in plain words: who wants what (a
 * connector, or a site a browser pilot wants to open), which answers it
 * offers, and what was chosen. Pure: the access card, the Inbox
 * summary and their tests read from these (`services/access.rs`
 * `AccessRequest` is the args' shape).
 */

import type { AccessChoice, GrantAccess } from '@/lib/bindings'

/** The approval tool a call without a grant asks through */
export const ACCESS_TOOL = 'connector_access'

export type AccessFamily = 'email' | 'calendar' | 'files' | 'connector' | 'site'

export interface AccessConnector {
  id: string
  name: string
}

/** A website a browser pilot asked to open (`SiteAccess` in Rust) */
export interface SiteAccess {
  /** As the browser sends it (punycode for an international name) */
  host: string
  /** As people read it (decoded) */
  display: string
  pilot: string
  /** Why the agent wants the page, in its words */
  reason: string | null
  /** Always allow can write it into the agent (not a pack's agent) */
  canSave: boolean
  /** As decided: the grant covers its subdomains too */
  subdomains: boolean
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
  /** For family `site` */
  site: SiteAccess | null
}

const FAMILIES: readonly AccessFamily[] = [
  'email',
  'calendar',
  'files',
  'connector',
  'site',
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

function parseSite(value: unknown): SiteAccess | null {
  if (typeof value !== 'object' || value === null) return null
  const raw = value as Record<string, unknown>
  const host = text(raw.host)
  if (!host) return null
  return {
    host,
    display: text(raw.display) || host,
    pilot: text(raw.pilot),
    reason: text(raw.reason) || null,
    canSave: raw.canSave === true,
    subdomains: raw.subdomains === true,
  }
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
  const site = parseSite(raw.site)
  if (family === 'site' && !site) return null
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
    site,
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
    | 'site'
  values: { agent: string; names: string }
}

/** "<Agent> wants to read your email (Outlook)" and the like */
export function accessSentence(
  request: AccessRequest,
  agentName: string
): AccessSentence {
  if (request.site) {
    return {
      key: 'site',
      values: { agent: agentName, names: request.site.display },
    }
  }
  const access = request.access === 'write' ? 'Write' : 'Read'
  return {
    key: `${request.family}${access}` as AccessSentence['key'],
    values: {
      agent: agentName,
      names: request.connectors.map(c => c.name).join(', '),
    },
  }
}

/**
 * The answers offered: a policy that asks every time allows only once; a
 * site is allowed once or always (when the agent's file can be changed)
 */
export function accessChoices(request: AccessRequest): AccessChoice[] {
  if (request.site) {
    return request.site.canSave
      ? ['allowOnce', 'allowSaved', 'deny']
      : ['allowOnce', 'deny']
  }
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

/** The raw host when people read it differently (an international name) */
export function rawSiteHost(site: SiteAccess): string | null {
  return site.host === site.display ? null : site.host
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
