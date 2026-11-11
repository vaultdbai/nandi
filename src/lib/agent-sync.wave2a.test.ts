import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import type { AgentCard, TaskResult } from '@/lib/bindings'
import { handleNandiEvent, localMidnight } from './agent-sync'
import { queryClient } from './query-client'

/** Query keys invalidated so far */
const invalidated = (spy: ReturnType<typeof vi.spyOn>): unknown[] =>
  (spy.mock.calls as unknown[][]).map(
    ([filters]) => (filters as { queryKey: unknown }).queryKey
  )

const card: AgentCard = {
  id: 'peer/papers',
  name: 'Papers',
  description: '',
  instanceId: 'peer',
  endpoint: '',
  capabilities: [{ name: 'find-papers' }],
  acceptsDelegation: true,
  maxConcurrent: 1,
  currentLoad: 0,
  owner: { githubHandle: 'ada', displayName: 'Ada' },
  signature: null,
  issuedAt: 0,
}

const result: TaskResult = {
  taskId: 'task-1',
  providerAgent: 'peer/papers',
  output: null,
  status: 'completed',
  durationMs: 10,
  tokensIn: 0,
  tokensOut: 0,
  error: null,
  direction: 'outbound',
  capability: 'find-papers',
  requesterAgent: 'me/_user',
  providerOwner: 'ada',
  requesterOwner: null,
  input: null,
  startedAt: 0,
  endedAt: 10,
}

describe('agent-sync: wave-2A network events', () => {
  let invalidate: ReturnType<typeof vi.spyOn>

  beforeEach(() => {
    invalidate = vi.spyOn(queryClient, 'invalidateQueries')
  })

  afterEach(() => {
    invalidate.mockRestore()
    queryClient.clear()
  })

  it('relay://status invalidates the relay queries', () => {
    handleNandiEvent({
      event: 'relay://status',
      payload: {
        connected: true,
        state: 'connected',
        reason: null,
        instanceId: 'me',
        relayUrl: 'ws://relay.test',
        relayUrlSource: 'config',
        agentsOnline: 1,
        instancesOnline: 1,
        lastConnectedAt: null,
        lastHeartbeatAt: null,
      },
    })
    expect(invalidated(invalidate)).toEqual([['relay']])
  })

  it('relay://agent_online invalidates relay, capabilities, and shared-with-me', () => {
    handleNandiEvent({ event: 'relay://agent_online', payload: { card } })
    const keys = invalidated(invalidate)
    expect(keys).toContainEqual(['relay'])
    expect(keys).toContainEqual(['capabilities'])
    expect(keys).toContainEqual(['shared', 'withMe'])
  })

  it('team://changed invalidates teams and the shared views', () => {
    handleNandiEvent({ event: 'team://changed', payload: { teamId: 'org' } })
    const keys = invalidated(invalidate)
    expect(keys).toContainEqual(['teams'])
    expect(keys).toContainEqual(['shared'])
  })

  it('delegation://task_completed invalidates delegation and the shared views', () => {
    handleNandiEvent({
      event: 'delegation://task_completed',
      payload: { result },
    })
    const keys = invalidated(invalidate)
    expect(keys).toContainEqual(['delegation'])
    expect(keys).toContainEqual(['shared'])
  })

  it('resync invalidates every query', () => {
    handleNandiEvent({ event: 'nandi://resync', payload: { missed: 1 } })
    // No filter: all queries, whatever their key
    expect(invalidate).toHaveBeenCalledWith()
  })
})

describe('localMidnight', () => {
  it('is the start of the local day', () => {
    const midnight = new Date(localMidnight())
    expect(midnight.getHours()).toBe(0)
    expect(midnight.getMinutes()).toBe(0)
    expect(Date.now() - midnight.getTime()).toBeLessThan(24 * 60 * 60 * 1000)
  })
})
