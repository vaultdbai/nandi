import { describe, expect, it } from 'vitest'
import { agentManifest, packManifest } from '@/test/fixtures'
import type { ConnectorDetails } from '@/hooks/useConnectors'
import type { ManifestCapabilities, ToolEntry } from '@/lib/bindings'
import {
  agentItem,
  applyKind,
  builtinToolItem,
  canDoOptions,
  connectorAction,
  connectorItem,
  connectorPill,
  approvedAtInstall,
  filterItems,
  itemsForTab,
  kindCounts,
  oldestFetch,
  packItem,
  permissionPhrases,
  pilotPill,
  startBlocked,
  unionCapabilities,
  workflowItems,
} from './library-model'
import { toolLabel } from './library-format'

const caps = (
  overrides: Partial<ManifestCapabilities> = {}
): ManifestCapabilities => ({
  tools: [],
  network: [],
  pilots: [],
  requiresApproval: [],
  aws: [],
  connectors: [],
  ...overrides,
})

function connector(
  overrides: Partial<ConnectorDetails> = {}
): ConnectorDetails {
  return {
    id: 'web_search',
    name: 'Web search',
    description: '',
    kind: 'mcp',
    status: { installed: true, running: false, healthy: false, error: null },
    secrets: [],
    setupHint: null,
    ...overrides,
  } as ConnectorDetails
}

function tool(id: string, enabled = true): ToolEntry {
  return {
    definition: {
      id,
      name: id,
      description: '',
      source: { kind: 'builtin' },
      inputSchema: null,
      requiresApproval: false,
      costEstimate: null,
      latencyClass: 'instant',
      tags: [],
    },
    enabled,
    usageCount: 0,
  }
}

describe('permissionPhrases', () => {
  it('says nothing for no special access', () => {
    expect(permissionPhrases(caps())).toEqual([])
  })

  it('lists access in the order the card shows it', () => {
    expect(
      permissionPhrases(
        caps({
          tools: ['delegate_to_agent', 'shell', 'read_file'],
          network: ['arxiv.org', 'arxiv.org', 'github.com'],
          pilots: ['browser'],
          aws: ['s3', 'sqs'],
        })
      )
    ).toEqual([
      { key: 'readsFiles' },
      { key: 'runsShell' },
      { key: 'asksAgents' },
      { key: 'internet', count: 2 },
      { key: 'pilots', names: 'browser' },
      { key: 'aws', names: 'S3, SQS' },
    ])
  })

  it('folds reading into writing', () => {
    expect(
      permissionPhrases(caps({ tools: ['read_file', 'write_file'] }))
    ).toEqual([{ key: 'readsWritesFiles' }])
  })
})

describe('consent', () => {
  it('approves at install only what the install dialog listed', () => {
    const requested = (key: string, items: string[]) => ({
      key,
      items,
      granted: false,
    })
    const shown = caps({ tools: ['shell'], connectors: ['filesystem'] })
    expect(
      approvedAtInstall(
        [
          requested('tools', ['shell']),
          requested('connectors', ['filesystem', 'github']),
          requested('folders', ['docs:write']),
          requested('delegation', ['accepts_delegation']),
        ],
        shown
      )
    ).toEqual([{ key: 'tools', items: ['shell'] }])
  })

  it('unions what a pack’s agents ask for', () => {
    expect(
      unionCapabilities([
        caps({ tools: ['shell'], network: ['a.org'], connectors: ['github'] }),
        caps({ tools: ['shell', 'read_file'], connectors: ['github', 'aws'] }),
      ])
    ).toEqual(
      caps({
        tools: ['shell', 'read_file'],
        network: ['a.org'],
        connectors: ['github', 'aws'],
      })
    )
  })
})

describe('status pills', () => {
  it('marks installed agents and newer versions', () => {
    expect(agentItem(agentManifest('a', 'A')).pill).toBeNull()
    expect(
      agentItem(agentManifest('a', 'A', { installedVersion: '1.2.0' })).pill
    ).toBe('installed')
    expect(
      agentItem(agentManifest('a', 'A', { installedVersion: '1.0.0' })).pill
    ).toBe('updateAvailable')
  })

  it('shows a built-in tool only when switched off', () => {
    expect(builtinToolItem(tool('shell')).pill).toBeNull()
    expect(builtinToolItem(tool('shell', false)).pill).toBe('off')
  })

  it('maps connector state', () => {
    expect(
      connectorPill(
        connector({
          status: {
            installed: false,
            running: false,
            healthy: false,
            error: null,
          },
        })
      )
    ).toBeNull()
    expect(
      connectorPill(
        connector({
          secrets: [
            { key: 'api_key', keyringEntry: 'k', isSet: false, required: true },
          ],
        })
      )
    ).toBe('needsSetup')
    // An optional secret (GitHub's token, when the GitHub CLI signs in)
    expect(
      connectorPill(
        connector({
          secrets: [
            { key: 'token', keyringEntry: 'k', isSet: false, required: false },
          ],
        })
      )
    ).toBe('stopped')
    expect(connectorPill(connector())).toBe('stopped')
    expect(
      connectorPill(
        connector({
          status: { installed: true, running: true, healthy: false },
        })
      )
    ).toBe('problem')
    expect(
      connectorPill(
        connector({ status: { installed: true, running: true, healthy: true } })
      )
    ).toBe('running')
  })

  it('maps pilot state', () => {
    expect(pilotPill(undefined, false)).toBe('notForThisComputer')
    expect(pilotPill(undefined, true)).toBeNull()
    const status = { installed: true, running: true, healthy: false, pid: 1 }
    expect(pilotPill(status, true)).toBe('problem')
    expect(pilotPill({ ...status, running: false }, true)).toBe('stopped')
  })
})

describe('connector rules', () => {
  it('says why Start is blocked', () => {
    expect(
      startBlocked(
        connector({
          secrets: [
            { key: 'api_key', keyringEntry: 'k', isSet: false, required: true },
          ],
        })
      )
    ).toBe('secrets')
    expect(
      startBlocked(connector({ setupHint: 'Choose the folder agents can use' }))
    ).toBe('setup')
    expect(startBlocked(connector())).toBeNull()
  })

  it('offers one action per state', () => {
    expect(
      connectorAction(
        connector({
          status: {
            installed: false,
            running: false,
            healthy: false,
          },
        })
      )
    ).toBe('install')
    expect(connectorAction(connector())).toBe('start')
    expect(
      connectorAction(
        connector({ status: { installed: true, running: true, healthy: true } })
      )
    ).toBe('stop')
  })
})

describe('listing', () => {
  const research = packManifest('research', 'Research', {
    installedVersion: '1.0.0',
    tags: ['papers'],
  })
  const items = [
    agentItem(agentManifest('writer', 'Writer', { tags: ['drafting'] })),
    packItem(research),
    ...workflowItems([research]),
    builtinToolItem(tool('shell')),
    connectorItem(connector()),
  ]

  it('keeps Installed to what is on this computer', () => {
    expect(itemsForTab('installed', items).map(i => i.key)).toEqual([
      'pack:research',
      'workflow:research-flow',
      'builtinTool:shell',
      'connector:web_search',
    ])
  })

  it('shows everything on Browse', () => {
    expect(itemsForTab('browse', items)).toHaveLength(items.length)
  })

  it('searches every word, including the pack a workflow comes in', () => {
    expect(
      filterItems(items, { query: 'research flow', canDo: null }).map(
        i => i.key
      )
    ).toEqual(['workflow:research-flow'])
  })

  it('filters by Can do, then kind, and counts per kind', () => {
    const searched = filterItems(items, { query: '', canDo: 'Papers' })
    expect(searched.map(i => i.key)).toEqual([
      'pack:research',
      'workflow:research-flow',
    ])
    expect(kindCounts(searched)).toEqual({
      all: 2,
      agents: 0,
      workflows: 1,
      tools: 0,
      packs: 1,
    })
    expect(applyKind(searched, 'packs').map(i => i.key)).toEqual([
      'pack:research',
    ])
  })

  it('offers each capability once, sorted', () => {
    expect(canDoOptions(items, ['summarize', 'papers'])).toEqual([
      'drafting',
      'papers',
      'summarize',
    ])
  })

  it('reports the oldest fetch', () => {
    expect(oldestFetch([null])).toBeNull()
    expect(
      oldestFetch(['2026-10-02T12:00:00Z', '2026-10-01T12:00:00Z', null])
    ).toBe(Date.parse('2026-10-01T12:00:00Z'))
  })
})

describe('toolLabel', () => {
  it('names each built-in by its friendly label key', () => {
    expect(toolLabel('read_file')).toBe('readFile')
    expect(toolLabel('delegate_to_agent')).toBe('delegate')
    expect(toolLabel('custom_tool')).toBe('custom_tool')
  })
})
