import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { AgentInfo, EgressLogEntry, EgressStatus } from '@/lib/bindings'
import { InternetAccessSection } from './InternetAccessSection'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

const agent = (id: string, name: string, kind: 'local' | 'pack') =>
  ({
    id,
    name,
    origin: kind === 'local' ? { kind } : { kind, pack: 'starter' },
  }) as AgentInfo

const blocked = (agentId: string, target: string): EgressLogEntry => ({
  ts: Date.now() / 1000,
  agentId,
  kind: 'fetch',
  target,
  allowed: false,
  status: null,
  durationMs: null,
  error: 'not allowed',
})

const definition = (id: string, network: string[]) => ({
  id,
  name: id,
  description: '',
  skillBody: 'Help.',
  instructions: '',
  modelProvider: null,
  modelName: null,
  image: null,
  memoryMb: null,
  cpu: null,
  capabilities: { tools: [], network, pilots: [] },
  runtime: 'native',
  runtimeModel: null,
  readOnly: false,
  networkRules: network.map(host => ({ host, methods: [] })),
})

describe('InternetAccessSection', () => {
  const original: Commands = {}
  let sites: Record<string, string[]>

  beforeEach(() => {
    for (const name of [
      'listAgents',
      'getEgressStatus',
      'repoLoadEntity',
      'repoSaveAgent',
    ])
      original[name] = mocked[name]
    sites = { writer: ['*'], scout: [] }
    mocked.listAgents = vi
      .fn()
      .mockResolvedValue(
        ok([
          agent('writer', 'Writer', 'local'),
          agent('scout', 'Scout', 'local'),
          agent('packed', 'Packed', 'pack'),
        ])
      )
    mocked.getEgressStatus = vi.fn().mockResolvedValue(
      ok({
        proxyUrl: 'http://127.0.0.1:11435',
        routes: [],
        aws: {
          configured: false,
          accessKeyHint: null,
          region: 'us-east-1',
          activeSessions: 0,
        },
        recentEntries: [
          blocked('scout', 'GET https://api.github.com/repos'),
          blocked('packed', 'GET https://pypi.org/simple'),
        ],
      } satisfies EgressStatus)
    )
    mocked.repoLoadEntity = vi.fn((_kind: string, id: string) =>
      Promise.resolve(ok(definition(id, sites[id] ?? [])))
    )
    mocked.repoSaveAgent = vi
      .fn()
      .mockResolvedValue(ok(agent('scout', 'Scout', 'local')))
  })

  afterEach(() => {
    Object.assign(mocked, original)
  })

  it('says when an agent may reach any site', async () => {
    render(<InternetAccessSection />)
    expect(await screen.findByText('Any site')).toBeInTheDocument()
    expect(await screen.findAllByText('No internet access')).not.toHaveLength(0)
  })

  it('allows a blocked site for the agent that asked', async () => {
    render(<InternetAccessSection />)
    const row = (
      await screen.findByText('GET https://api.github.com/repos')
    ).closest('li')
    if (!row) throw new Error('no row')
    await userEvent.click(within(row).getByRole('button', { name: 'Allow' }))

    await waitFor(() => expect(mocked.repoSaveAgent).toHaveBeenCalled())
    const [id, input] =
      vi.mocked(mocked.repoSaveAgent as (...args: unknown[]) => unknown).mock
        .calls[0] ?? []
    expect(id).toBe('scout')
    expect(input).toMatchObject({
      capabilities: { network: ['api.github.com'] },
    })
  })

  it('offers no Allow for installed agents', async () => {
    render(<InternetAccessSection />)
    const row = (
      await screen.findByText('GET https://pypi.org/simple')
    ).closest('li')
    if (!row) throw new Error('no row')
    expect(
      within(row).queryByRole('button', { name: 'Allow' })
    ).not.toBeInTheDocument()
  })
})
