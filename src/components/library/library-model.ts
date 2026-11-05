/**
 * Library (task 11): one card model for everything an agent can be given:
 * marketplace agents and packs, the workflows packs bring, built-in tools,
 * connectors and pilots. Pure: the view feeds it query data and renders
 * what it returns.
 */
import type {
  AgentManifest,
  ManifestCapabilities,
  PackManifest,
  PilotDefinition,
  PilotStatus,
  ToolEntry,
} from '@/lib/bindings'
import type { ConnectorDetails } from '@/hooks/useConnectors'
import { compareVersions } from '@/lib/marketplace-format'

export type LibraryTabId = 'installed' | 'browse' | 'shared'
export type LibraryKind = 'agents' | 'workflows' | 'tools' | 'packs'
export type KindFilter = 'all' | LibraryKind

export const LIBRARY_KINDS: LibraryKind[] = [
  'agents',
  'workflows',
  'tools',
  'packs',
]

/** At most one per card (§4.2); `null` on an item that isn't installed */
export type StatusPill =
  | 'installed'
  | 'running'
  | 'stopped'
  | 'needsSetup'
  | 'problem'
  | 'updateAvailable'
  | 'off'
  | 'notForThisComputer'

/** One phrase of the permissions line, in the order §4.2 lists them */
export type PermissionPhrase =
  | { key: 'readsFiles' }
  | { key: 'readsWritesFiles' }
  | { key: 'runsShell' }
  | { key: 'asksAgents' }
  | { key: 'internet'; count: number }
  | { key: 'pilots'; names: string }
  | { key: 'aws'; names: string }

export type ItemType =
  'agent' | 'pack' | 'workflow' | 'builtinTool' | 'connector' | 'pilot'

export interface LibraryItem {
  /** `<type>:<id>`, the drawer's `item` */
  key: string
  type: ItemType
  kind: LibraryKind
  id: string
  name: string
  description: string
  tags: string[]
  /** What it can do for others (tags until manifests declare `provides`) */
  canDo: string[]
  /** `null` for tools: their card says how they behave instead */
  permissions: PermissionPhrase[] | null
  pill: StatusPill | null
  installed: boolean
  publisher: string | null
  version: string | null
  verified: boolean
  compatible: boolean
  /** The pack a workflow comes in */
  packName?: string
  packId?: string
}

/** The access an item asks for, as one line (empty: "No special access") */
export function permissionPhrases(
  capabilities: ManifestCapabilities
): PermissionPhrase[] {
  const { tools, network, pilots, aws } = capabilities
  const phrases: PermissionPhrase[] = []
  if (tools.includes('write_file')) {
    phrases.push({ key: 'readsWritesFiles' })
  } else if (tools.includes('read_file') || tools.includes('list_dir')) {
    phrases.push({ key: 'readsFiles' })
  }
  if (tools.includes('shell')) phrases.push({ key: 'runsShell' })
  if (tools.includes('delegate_to_agent')) phrases.push({ key: 'asksAgents' })
  if (network.length > 0) {
    phrases.push({ key: 'internet', count: new Set(network).size })
  }
  if (pilots.length > 0) {
    phrases.push({ key: 'pilots', names: pilots.join(', ') })
  }
  if (aws.length > 0) {
    phrases.push({
      key: 'aws',
      names: aws.map(s => s.toUpperCase()).join(', '),
    })
  }
  return phrases
}

/** Every access category an item asks for, for the consent it grants */
export function consentCategories(
  capabilities: ManifestCapabilities
): string[] {
  const categories: string[] = []
  if (capabilities.tools.length > 0) categories.push('tools')
  if (capabilities.network.length > 0) categories.push('network')
  if (capabilities.aws.length > 0) categories.push('aws')
  if (capabilities.pilots.length > 0) categories.push('pilots')
  return categories
}

/** Everything a pack's agents together ask for */
export function unionCapabilities(
  list: ManifestCapabilities[]
): ManifestCapabilities {
  const union = (pick: (c: ManifestCapabilities) => string[]) => [
    ...new Set(list.flatMap(pick)),
  ]
  return {
    tools: union(c => c.tools),
    network: union(c => c.network),
    pilots: union(c => c.pilots),
    requiresApproval: union(c => c.requiresApproval),
    aws: union(c => c.aws),
    connectors: union(c => c.connectors),
  }
}

function versionPill(
  installedVersion: string | null,
  latestVersion: string
): StatusPill | null {
  if (installedVersion === null) return null
  return compareVersions(latestVersion, installedVersion) > 0
    ? 'updateAvailable'
    : 'installed'
}

export function agentItem(agent: AgentManifest): LibraryItem {
  return {
    key: `agent:${agent.id}`,
    type: 'agent',
    kind: 'agents',
    id: agent.id,
    name: agent.name,
    description: agent.description,
    tags: agent.tags,
    canDo: agent.tags,
    permissions: permissionPhrases(agent.capabilities),
    pill: versionPill(agent.installedVersion, agent.latestVersion),
    installed: agent.installedVersion !== null,
    publisher: agent.publisher,
    version: agent.installedVersion ?? agent.latestVersion,
    verified: agent.verified,
    compatible: agent.compatible,
  }
}

export function packItem(pack: PackManifest): LibraryItem {
  return {
    key: `pack:${pack.id}`,
    type: 'pack',
    kind: 'packs',
    id: pack.id,
    name: pack.name,
    description: pack.description,
    tags: pack.tags,
    canDo: pack.tags,
    permissions: permissionPhrases(pack.capabilities),
    pill: versionPill(pack.installedVersion, pack.latestVersion),
    installed: pack.installedVersion !== null,
    publisher: pack.publisher,
    version: pack.installedVersion ?? pack.latestVersion,
    verified: pack.verified,
    compatible: pack.compatible,
  }
}

/** The workflows packs bring: installed with their pack, opened from it */
export function workflowItems(packs: PackManifest[]): LibraryItem[] {
  return packs.flatMap(pack =>
    pack.workflows.map(workflow => ({
      key: `workflow:${workflow}`,
      type: 'workflow' as const,
      kind: 'workflows' as const,
      id: workflow,
      name: workflow,
      description: '',
      tags: pack.tags,
      canDo: pack.tags,
      permissions: null,
      pill: pack.installedVersion === null ? null : ('installed' as const),
      installed: pack.installedVersion !== null,
      publisher: pack.publisher,
      version: null,
      verified: pack.verified,
      compatible: pack.compatible,
      packName: pack.name,
      packId: pack.id,
    }))
  )
}

/** The five built-ins get a friendly label key; others keep their name */
export const BUILTIN_TOOL_LABELS: Record<string, string> = {
  shell: 'shell',
  read_file: 'readFile',
  write_file: 'writeFile',
  list_dir: 'listDir',
  delegate_to_agent: 'delegate',
}

export function builtinToolItem(entry: ToolEntry): LibraryItem {
  const { definition } = entry
  return {
    key: `builtinTool:${definition.id}`,
    type: 'builtinTool',
    kind: 'tools',
    id: definition.id,
    name: definition.name,
    description: definition.description,
    tags: definition.tags,
    canDo: [],
    permissions: null,
    pill: entry.enabled ? null : 'off',
    installed: true,
    publisher: null,
    version: null,
    verified: true,
    compatible: true,
  }
}

/** Why Start can't run yet, if anything (§4.5) */
export type StartBlocked = 'secrets' | 'setup'

export function startBlocked(connector: ConnectorDetails): StartBlocked | null {
  if (
    (connector.secrets ?? []).some(secret => secret.required && !secret.isSet)
  ) {
    return 'secrets'
  }
  if (connector.setupHint) return 'setup'
  return null
}

export function connectorPill(connector: ConnectorDetails): StatusPill | null {
  const { status } = connector
  if (!status.installed) return null
  if (!status.running && startBlocked(connector) !== null) return 'needsSetup'
  if (status.running && !status.healthy) return 'problem'
  return status.running ? 'running' : 'stopped'
}

/** The card's one button for a connector (`null`: none) */
export type ConnectorAction = 'install' | 'setUp' | 'start' | 'stop'

export function connectorAction(
  connector: ConnectorDetails
): ConnectorAction | null {
  if (!connector.status.installed) return 'install'
  if (connector.status.running) return 'stop'
  return startBlocked(connector) === null ? 'start' : 'setUp'
}

export function connectorItem(connector: ConnectorDetails): LibraryItem {
  return {
    key: `connector:${connector.id}`,
    type: 'connector',
    kind: 'tools',
    id: connector.id,
    name: connector.name,
    description: connector.description,
    tags: [],
    canDo: [],
    permissions: null,
    pill: connectorPill(connector),
    installed: connector.status.installed,
    publisher: null,
    version: null,
    verified: true,
    compatible: true,
  }
}

export function pilotPill(
  status: PilotStatus | undefined,
  runsHere: boolean
): StatusPill | null {
  if (!runsHere) return 'notForThisComputer'
  if (!status?.installed) return null
  if (status.running && !status.healthy) return 'problem'
  return status.running ? 'running' : 'stopped'
}

export function pilotItem(
  pilot: PilotDefinition,
  status: PilotStatus | undefined,
  runsHere: boolean
): LibraryItem {
  return {
    key: `pilot:${pilot.id}`,
    type: 'pilot',
    kind: 'tools',
    id: pilot.id,
    name: pilot.name,
    description: pilot.description,
    tags: [],
    canDo: [],
    permissions: null,
    pill: pilotPill(status, runsHere),
    installed: status?.installed ?? false,
    publisher: null,
    version: null,
    verified: true,
    compatible: runsHere,
  }
}

/**
 * The items a tab shows: Installed has what's on this computer (built-in
 * tools always); Browse has everything.
 */
export function itemsForTab(
  tab: Exclude<LibraryTabId, 'shared'>,
  items: LibraryItem[]
): LibraryItem[] {
  if (tab === 'installed') {
    return items.filter(item => item.installed)
  }
  return items
}

export interface LibraryFilter {
  kind: KindFilter
  query: string
  canDo: string | null
}

function matchesQuery(item: LibraryItem, query: string): boolean {
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return true
  const haystack = [
    item.name,
    item.description,
    item.packName ?? '',
    ...item.tags,
    ...item.canDo,
  ]
    .join(' ')
    .toLowerCase()
  return words.every(word => haystack.includes(word))
}

/** Search and Can do; the kind chip applies on top (`applyKind`) */
export function filterItems(
  items: LibraryItem[],
  filter: Omit<LibraryFilter, 'kind'>
): LibraryItem[] {
  const canDo = filter.canDo?.toLowerCase() ?? null
  return items.filter(
    item =>
      matchesQuery(item, filter.query) &&
      (canDo === null || item.canDo.some(c => c.toLowerCase() === canDo))
  )
}

export function applyKind(
  items: LibraryItem[],
  kind: KindFilter
): LibraryItem[] {
  return kind === 'all' ? items : items.filter(item => item.kind === kind)
}

/** Chip counts after search (§4.1) */
export function kindCounts(items: LibraryItem[]): Record<KindFilter, number> {
  const counts: Record<KindFilter, number> = {
    all: items.length,
    agents: 0,
    workflows: 0,
    tools: 0,
    packs: 0,
  }
  for (const item of items) counts[item.kind] += 1
  return counts
}

/** The Can do options: every capability or tag offered, sorted */
export function canDoOptions(
  items: LibraryItem[],
  shared: string[] = []
): string[] {
  return [...new Set([...items.flatMap(item => item.canDo), ...shared])].sort(
    (a, b) => a.localeCompare(b)
  )
}

/** The oldest fetch across sources, for "Updated 5 min ago" (§4.1) */
export function oldestFetch(fetchedAt: (string | null)[]): number | null {
  const times = fetchedAt
    .map(at => (at ? Date.parse(at) : Number.NaN))
    .filter(Number.isFinite)
  return times.length === 0 ? null : Math.min(...times)
}

/** Sources older than this read as stale (amber) */
export const STALE_SOURCES_MS = 24 * 60 * 60 * 1000
