/**
 * Registry connectors and their install consent, in plain words: install
 * or update, where it runs, how it signs in, and its tools by access.
 * Pure (`marketplace/connectors.rs` builds the consent).
 */

import type {
  AuthSpec,
  ConnectorEntry,
  PackageInfo,
  PackageTool,
} from '@/lib/bindings'

export type EntryState = 'install' | 'update' | 'installed'

export function entryState(entry: {
  version: string
  installedVersion: string | null
}): EntryState {
  if (entry.installedVersion === null) return 'install'
  return entry.installedVersion === entry.version ? 'installed' : 'update'
}

/** The host of an https URL, else null */
export function urlHost(text: string): string | null {
  if (!text.startsWith('https://')) return null
  try {
    return new URL(text).host || null
  } catch {
    return null
  }
}

/** Under `connectorConsent.runs.` */
export type RunsWhere =
  | { key: 'container' }
  | { key: 'image'; image: string }
  | { key: 'server'; host: string }
  | { key: 'host' }

export function runsWhere(
  pkg: Pick<PackageInfo, 'runs' | 'image' | 'placement'>
): RunsWhere {
  const host = urlHost(pkg.runs)
  if (host) return { key: 'server', host }
  if (pkg.placement === 'host') return { key: 'host' }
  if (pkg.image) return { key: 'image', image: pkg.image }
  return { key: 'container' }
}

export type SignIn =
  | { kind: 'none' }
  | { kind: 'apiKey'; label: string | null; env: string[] }
  /** OAuth or a device code: not something this dialog can do yet */
  | { kind: 'unsupported' }

export function signIn(auth: AuthSpec): SignIn {
  switch (auth.kind) {
    case 'none':
      return { kind: 'none' }
    case 'api_key':
      return { kind: 'apiKey', label: auth.label, env: auth.env }
    default:
      return { kind: 'unsupported' }
  }
}

/** Install is possible once a key is typed (when one is needed) */
export function canInstall(auth: AuthSpec, apiKey: string): boolean {
  const need = signIn(auth)
  if (need.kind === 'unsupported') return false
  return need.kind === 'none' || apiKey.trim() !== ''
}

export function toolsByAccess(tools: PackageTool[]): {
  read: PackageTool[]
  write: PackageTool[]
} {
  return {
    read: tools.filter(tool => tool.access === 'read'),
    write: tools.filter(tool => tool.access !== 'read'),
  }
}

/** Entries matching the search, by name */
export function filterEntries(
  entries: ConnectorEntry[],
  search: string
): ConnectorEntry[] {
  const query = search.trim().toLowerCase()
  return entries
    .filter(
      entry =>
        !query ||
        [entry.name, entry.description, entry.publisher, ...entry.tags].some(
          text => text.toLowerCase().includes(query)
        )
    )
    .sort((a, b) => a.name.localeCompare(b.name))
}

/** The ids a pack needs that aren't installed, with their entries */
export function missingConnectors(
  required: string[],
  installedIds: string[],
  entries: ConnectorEntry[]
): { id: string; entry: ConnectorEntry | null }[] {
  return required
    .filter(id => !installedIds.includes(id))
    .map(id => ({ id, entry: entries.find(e => e.id === id) ?? null }))
}
