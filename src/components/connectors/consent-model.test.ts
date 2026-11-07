import { describe, expect, it } from 'vitest'
import type { AuthSpec, ConnectorEntry } from '@/lib/bindings'
import {
  canInstall,
  entryState,
  filterEntries,
  missingConnectors,
  runsWhere,
  signIn,
  toolsByAccess,
  urlHost,
} from './consent-model'

const auth = (kind: AuthSpec['kind']): AuthSpec => ({
  kind,
  env: ['TRACKER_TOKEN'],
  scopes: [],
  label: 'Tracker API key',
})

function entry(overrides: Partial<ConnectorEntry> = {}): ConnectorEntry {
  return {
    id: 'tracker',
    name: 'Issue tracker',
    description: 'Read and file issues.',
    publisher: 'acme',
    repo: 'https://github.com/acme/registry',
    path: 'connectors/tracker',
    version: '1.0.0',
    gitRef: 'tracker-v1.0.0',
    commit: null,
    signature: null,
    kind: 'mcp',
    placement: 'container',
    image: null,
    tags: ['issues'],
    reads: [],
    writes: [],
    hosts: [],
    auth: 'api_key',
    registry: 'https://github.com/acme/registry',
    verified: false,
    installedVersion: null,
    ...overrides,
  }
}

describe('entryState', () => {
  it('says install, update or installed', () => {
    expect(entryState(entry())).toBe('install')
    expect(entryState(entry({ installedVersion: '0.9.0' }))).toBe('update')
    expect(entryState(entry({ installedVersion: '1.0.0' }))).toBe('installed')
  })
})

describe('runsWhere', () => {
  it('names the container, an image, a server or this computer', () => {
    const pkg = { runs: 'npx -y @acme/tracker@1.2.3', image: null }
    expect(runsWhere({ ...pkg, placement: 'container' })).toEqual({
      key: 'container',
    })
    expect(
      runsWhere({ ...pkg, image: 'ghcr.io/acme/t:1', placement: 'container' })
    ).toEqual({ key: 'image', image: 'ghcr.io/acme/t:1' })
    expect(
      runsWhere({
        runs: 'https://mcp.acme.example/sse',
        image: null,
        placement: 'container',
      })
    ).toEqual({ key: 'server', host: 'mcp.acme.example' })
    expect(runsWhere({ ...pkg, placement: 'host' })).toEqual({ key: 'host' })
  })

  it('reads hosts only from https URLs', () => {
    expect(urlHost('http://x.example')).toBeNull()
    expect(urlHost('https://x.example:8443/a')).toBe('x.example:8443')
  })
})

describe('signIn and canInstall', () => {
  it('needs a key for api_key, nothing for none', () => {
    expect(signIn(auth('api_key'))).toEqual({
      kind: 'apiKey',
      label: 'Tracker API key',
      env: ['TRACKER_TOKEN'],
    })
    expect(canInstall(auth('api_key'), ' ')).toBe(false)
    expect(canInstall(auth('api_key'), 'k')).toBe(true)
    expect(canInstall(auth('none'), '')).toBe(true)
  })

  it("can't install what needs OAuth or a device code yet", () => {
    expect(signIn(auth('oauth'))).toEqual({ kind: 'unsupported' })
    expect(canInstall(auth('device_code'), 'k')).toBe(false)
  })
})

describe('toolsByAccess', () => {
  it('splits read and write tools', () => {
    const tools = toolsByAccess([
      { name: 'list', description: '', access: 'read' },
      { name: 'create', description: '', access: 'write' },
    ])
    expect(tools.read.map(t => t.name)).toEqual(['list'])
    expect(tools.write.map(t => t.name)).toEqual(['create'])
  })
})

describe('filterEntries', () => {
  it('matches name, description, publisher and tags', () => {
    const entries = [
      entry(),
      entry({ id: 'cal', name: 'Agenda', tags: [], description: 'x' }),
    ]
    expect(filterEntries(entries, 'ISSUES').map(e => e.id)).toEqual(['tracker'])
    expect(filterEntries(entries, '').map(e => e.id)).toEqual([
      'cal',
      'tracker',
    ])
  })
})

describe('missingConnectors', () => {
  it('lists required connectors not installed, with entries', () => {
    expect(
      missingConnectors(['tracker', 'github', 'nowhere'], ['github'], [entry()])
    ).toEqual([
      { id: 'tracker', entry: entry() },
      { id: 'nowhere', entry: null },
    ])
  })
})
