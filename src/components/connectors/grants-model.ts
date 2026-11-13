/**
 * Connector grants for display: grouped by agent, named, and per connector
 * on an agent's page. Pure (`domain/grant.rs` is the source of truth).
 */

import type { GrantAccess, GrantInfo } from '@/lib/bindings'

/** Product names: not translated */
export const ACCOUNT_NAMES: Record<string, string> = {
  outlook: 'Outlook',
  gmail: 'Gmail',
  'outlook-calendar': 'Outlook calendar',
  'google-calendar': 'Google Calendar',
  onedrive: 'OneDrive',
  'google-drive': 'Google Drive',
}

export interface GrantRow {
  grant: GrantInfo
  connectorName: string
}

export interface GrantGroup {
  agentId: string
  agentName: string
  rows: GrantRow[]
}

export interface GrantNames {
  agent: (id: string) => string | undefined
  connector: (id: string) => string | undefined
}

export function connectorName(id: string, names: GrantNames): string {
  return names.connector(id) ?? ACCOUNT_NAMES[id] ?? id
}

const ACCESS_ORDER: Record<GrantAccess, number> = { read: 0, write: 1 }

/** Grants per agent, agents and connectors by name, read before write */
export function groupGrants(
  grants: GrantInfo[],
  names: GrantNames
): GrantGroup[] {
  const groups = new Map<string, GrantGroup>()
  for (const grant of grants) {
    const group = groups.get(grant.agent) ?? {
      agentId: grant.agent,
      agentName: names.agent(grant.agent) ?? grant.agent,
      rows: [],
    }
    group.rows.push({
      grant,
      connectorName: connectorName(grant.connector, names),
    })
    groups.set(grant.agent, group)
  }
  const sorted = [...groups.values()].sort((a, b) =>
    a.agentName.localeCompare(b.agentName)
  )
  for (const group of sorted) {
    group.rows.sort(
      (a, b) =>
        a.connectorName.localeCompare(b.connectorName) ||
        ACCESS_ORDER[a.grant.access] - ACCESS_ORDER[b.grant.access] ||
        a.grant.scope.localeCompare(b.grant.scope)
    )
  }
  return sorted
}

/** An agent's grants on one connector, per access (saved first) */
export function agentGrants(
  grants: GrantInfo[],
  agentId: string,
  connectorId: string
): Record<GrantAccess, GrantInfo[]> {
  const mine = grants.filter(
    g => g.agent === agentId && g.connector === connectorId
  )
  const pick = (access: GrantAccess) =>
    mine
      .filter(g => g.access === access)
      .sort((a, b) => (a.scope === b.scope ? 0 : a.scope === 'saved' ? -1 : 1))
  return { read: pick('read'), write: pick('write') }
}

/**
 * Connectors an agent can be given access to on its page: installed
 * package connectors and connected accounts, by id
 */
export function grantableConnectors(
  connectors: { id: string; name: string; package?: unknown }[],
  connectedAccounts: string[]
): { id: string; name: string }[] {
  const packages = connectors
    .filter(c => c.package !== null && c.package !== undefined)
    .map(c => ({ id: c.id, name: c.name }))
  const accounts = connectedAccounts
    .filter(id => !packages.some(p => p.id === id))
    .map(id => ({ id, name: ACCOUNT_NAMES[id] ?? id }))
  return [...packages, ...accounts]
}
