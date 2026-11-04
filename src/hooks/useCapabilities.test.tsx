import { describe, it, expect, vi, beforeEach } from 'vitest'
import type { ReactNode } from 'react'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { renderHook, waitFor } from '@testing-library/react'
import type { AgentCard } from '@/lib/bindings'
import {
  capabilityQueryKeys,
  useAgentCard,
  useAgentsByCapability,
  useCapabilities,
  useCapabilityEvents,
  useCapabilityListings,
  useTransferCapabilityOwnership,
  isCapabilityEvent,
} from './useCapabilities'

const mocks = vi.hoisted(() => ({
  listDeclaredCapabilities: vi.fn(),
  findAgentsByCapability: vi.fn(),
  getAgentCard: vi.fn(),
  listCapabilities: vi.fn(),
  transferCapabilityOwnership: vi.fn(),
  subscribe: vi.fn(),
}))

vi.mock('@/lib/client', () => ({ client: mocks }))

const card: AgentCard = {
  id: 'inst/papers',
  name: 'Papers',
  description: '',
  instanceId: 'inst',
  endpoint: '',
  capabilities: [{ name: 'find-papers', description: '' }],
  acceptsDelegation: true,
  maxConcurrent: 1,
  currentLoad: 0,
  owner: { githubHandle: '', displayName: '' },
  signature: null,
  issuedAt: 0,
}

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  })
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>
}

describe('useCapabilities', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('lists declared capabilities', async () => {
    mocks.listDeclaredCapabilities.mockResolvedValueOnce({
      status: 'ok',
      data: ['find-papers', 'summarize-paper'],
    })
    const { result } = renderHook(() => useCapabilities(), { wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data).toEqual(['find-papers', 'summarize-paper'])
  })

  it('finds agents by capability, and waits for one', async () => {
    mocks.findAgentsByCapability.mockResolvedValueOnce({
      status: 'ok',
      data: [card],
    })
    const { result } = renderHook(
      () => useAgentsByCapability(' find-papers '),
      { wrapper }
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data).toEqual([card])
    expect(mocks.findAgentsByCapability).toHaveBeenCalledWith('find-papers')

    const idle = renderHook(() => useAgentsByCapability(null), { wrapper })
    expect(idle.result.current.fetchStatus).toBe('idle')
    expect(mocks.findAgentsByCapability).toHaveBeenCalledTimes(1)
  })

  it('loads an agent card and surfaces errors', async () => {
    mocks.getAgentCard.mockResolvedValueOnce({ status: 'ok', data: card })
    const { result } = renderHook(() => useAgentCard('papers'), { wrapper })
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(result.current.data?.id).toBe('inst/papers')

    mocks.getAgentCard.mockResolvedValueOnce({
      status: 'error',
      error: "Agent 'ghost' not found",
    })
    const failed = renderHook(() => useAgentCard('ghost'), { wrapper })
    await waitFor(() => expect(failed.result.current.isError).toBe(true))
  })

  it('keys everything under capabilities', () => {
    expect(capabilityQueryKeys.agents('x')[0]).toBe('capabilities')
    expect(capabilityQueryKeys.card('a')[0]).toBe('capabilities')
    expect(capabilityQueryKeys.evidence('x', 30)[0]).toBe('capabilities')
    expect(capabilityQueryKeys.ambiguities('x')[0]).toBe('capabilities')
  })
})

describe('capability definitions', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('treats capability, provider, evidence, registry, and catalog events as changes', () => {
    expect(isCapabilityEvent('capability://updated')).toBe(true)
    expect(isCapabilityEvent('provider://registered')).toBe(true)
    expect(isCapabilityEvent('evidence://aggregated')).toBe(true)
    expect(isCapabilityEvent('registry://synced')).toBe(true)
    expect(isCapabilityEvent('tool://catalog_changed')).toBe(true)
    expect(isCapabilityEvent('tool://usage_recorded')).toBe(false)
    expect(isCapabilityEvent('agent://token')).toBe(false)
  })

  it('refetches the listings on a capability event', async () => {
    type Handler = (event: { event: string; payload: unknown }) => void
    const handlers: Handler[] = []
    const emit: Handler = event => handlers.forEach(handler => handler(event))
    mocks.subscribe.mockImplementation((handler: Handler) => {
      handlers.push(handler)
      return { ready: Promise.resolve(), unsubscribe: vi.fn() }
    })
    mocks.listCapabilities.mockResolvedValue({ status: 'ok', data: [] })
    const { result } = renderHook(
      () => {
        useCapabilityEvents()
        return useCapabilityListings()
      },
      { wrapper }
    )
    await waitFor(() => expect(result.current.isSuccess).toBe(true))
    expect(mocks.listCapabilities).toHaveBeenCalledTimes(1)

    emit({ event: 'agent://token', payload: null })
    emit({ event: 'capability://created', payload: null })
    await waitFor(() => expect(mocks.listCapabilities).toHaveBeenCalledTimes(2))
  })

  it('transfers ownership with the new owner and reason', async () => {
    mocks.transferCapabilityOwnership.mockResolvedValue({
      status: 'ok',
      data: { id: 'cap' },
    })
    const { result } = renderHook(() => useTransferCapabilityOwnership(), {
      wrapper,
    })
    const owner = { kind: 'user' as const, id: 'alice', displayName: 'Alice' }
    await result.current.mutateAsync({
      id: 'cap',
      newOwner: owner,
      reason: null,
    })
    expect(mocks.transferCapabilityOwnership).toHaveBeenCalledWith(
      'cap',
      owner,
      null
    )
  })
})
