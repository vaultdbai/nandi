import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { run } from '@/test/fixtures'
import { handleNandiEvent } from './agent-sync'
import { queryClient } from './query-client'

/** Query keys invalidated so far */
const invalidated = (spy: ReturnType<typeof vi.spyOn>): unknown[] =>
  (spy.mock.calls as unknown[][]).map(
    ([filters]) => (filters as { queryKey: unknown }).queryKey
  )

describe('agent-sync: runs, audit, and wave-1 events', () => {
  let invalidate: ReturnType<typeof vi.spyOn>

  beforeEach(() => {
    invalidate = vi.spyOn(queryClient, 'invalidateQueries')
  })

  afterEach(() => {
    invalidate.mockRestore()
    vi.useRealTimers()
    queryClient.clear()
  })

  it('refreshes run history and the LLM log on run events (debounced)', () => {
    vi.useFakeTimers()
    const payload = {
      agentId: 'writer',
      conversationId: 'agent:writer',
      run: run('r1', 'writer'),
    }
    handleNandiEvent({ event: 'agent://run', payload })
    handleNandiEvent({ event: 'agent://run', payload })
    expect(invalidated(invalidate)).not.toContainEqual(['globalRuns'])

    vi.advanceTimersByTime(300)
    const keys = invalidated(invalidate)
    expect(
      keys.filter(k => JSON.stringify(k) === '["globalRuns"]')
    ).toHaveLength(1)
    expect(keys).toContainEqual(['auditLog', 'llm'])
  })

  it('refreshes run history on dashboard activity', () => {
    vi.useFakeTimers()
    handleNandiEvent({
      event: 'dashboard://activity',
      payload: {
        item: {
          id: 'a1',
          agentId: 'writer',
          agentName: 'Writer',
          action: 'completed',
          subject: 'Task',
          runId: 'r1',
          createdAt: 0,
        },
      },
    } as Parameters<typeof handleNandiEvent>[0])
    vi.advanceTimersByTime(300)
    expect(invalidated(invalidate)).toContainEqual(['globalRuns'])
  })

  it('refreshes the trigger log when a trigger fires', () => {
    handleNandiEvent({
      event: 'trigger://fired',
      payload: { triggerId: 't1' },
    } as Parameters<typeof handleNandiEvent>[0])
    expect(invalidated(invalidate)).toContainEqual(['auditLog', 'triggers'])
  })

  it('invalidates git status when a repo file changes', () => {
    handleNandiEvent({
      event: 'repo://file_changed',
      payload: {
        kind: 'agent',
        id: 'writer',
        sourcePath: 'agents/writer/skill.md',
        result: 'changed',
        changedBy: 'external',
      },
    })
    const keys = invalidated(invalidate)
    expect(keys).toContainEqual(['git'])
    expect(keys).toContainEqual(['repoEntity', 'agent', 'writer'])
    expect(keys).toContainEqual(['agent-policies', 'writer'])
  })

  it('refreshes every agent’s policies when a policy file changes', () => {
    const changed = (kind: 'policy' | 'workflow') =>
      handleNandiEvent({
        event: 'repo://file_changed',
        payload: {
          kind,
          id: 'finance-safe',
          sourcePath: '.nandi/policies/finance-safe.md',
          result: 'invalid',
          changedBy: 'watcher',
        },
      })
    changed('workflow')
    expect(invalidated(invalidate)).not.toContainEqual(['agent-policies'])
    changed('policy')
    expect(invalidated(invalidate)).toContainEqual(['agent-policies'])
  })

  it('refreshes every agent’s policies when Studio saves a policy', () => {
    const saved = (kind: 'policy' | 'workflow') =>
      handleNandiEvent({
        event: 'studio://artifact_saved',
        payload: {
          result: {
            artifact: { kind, id: 'finance-safe' },
            commit: 'abc123',
            pushedBranch: null,
            warnings: [],
          },
        },
      } as Parameters<typeof handleNandiEvent>[0])
    saved('workflow')
    expect(invalidated(invalidate)).not.toContainEqual(['agent-policies'])
    saved('policy')
    expect(invalidated(invalidate)).toContainEqual(['agent-policies'])
  })

  it('stores pilot and connector status, and refreshes their lists', () => {
    const pilotStatus = { running: true } as never
    handleNandiEvent({
      event: 'pilot://changed',
      payload: { pilotId: 'desktop', status: pilotStatus },
    })
    expect(queryClient.getQueryData(['pilotStatus', 'desktop'])).toBe(
      pilotStatus
    )
    expect(invalidated(invalidate)).toContainEqual(['pilots'])

    const connectorStatus = { connected: true } as never
    handleNandiEvent({
      event: 'connector://changed',
      payload: { connectorId: 'github', status: connectorStatus },
    })
    expect(queryClient.getQueryData(['connectorStatus', 'github'])).toBe(
      connectorStatus
    )
    expect(invalidated(invalidate)).toContainEqual(['connectors'])

    handleNandiEvent({ event: 'connectors://changed', payload: {} })
    expect(invalidated(invalidate)).toContainEqual(['connectorStatus'])
  })
})
