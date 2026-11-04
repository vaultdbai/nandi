import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import { handleNandiEvent } from './agent-sync'
import { notify } from './notifications'
import { queryClient } from './query-client'

vi.mock('./notifications', () => ({ notify: vi.fn() }))

/** Query keys invalidated so far */
const invalidated = (spy: ReturnType<typeof vi.spyOn>): unknown[] =>
  (spy.mock.calls as unknown[][]).map(
    ([filters]) => (filters as { queryKey: unknown }).queryKey
  )

describe('agent-sync: wave EF events', () => {
  let invalidate: ReturnType<typeof vi.spyOn>

  beforeEach(() => {
    invalidate = vi.spyOn(queryClient, 'invalidateQueries')
    vi.mocked(notify).mockClear()
  })

  afterEach(() => {
    invalidate.mockRestore()
    queryClient.clear()
  })

  it('says when a workflow starts waiting on you, outside the app too', () => {
    handleNandiEvent({
      event: 'workflow://manual_step_ready',
      payload: {
        runId: 'run-1',
        workflowId: 'onboarding',
        workflowName: 'Onboarding Aayushi',
        stepId: 'laptop',
        instructions: 'Add the laptop tag',
        deadline: null,
        readyAt: 1,
      },
    })
    expect(invalidated(invalidate)).toContainEqual(['workflow', 'onboarding'])
    expect(notify).toHaveBeenCalledWith(
      'Onboarding Aayushi needs you',
      'Add the laptop tag',
      { native: true }
    )
  })

  it('tells the requester who took their task, but not the provider', () => {
    const reassigned = {
      taskId: 'task-1',
      fromInstance: 'alice-1',
      toInstance: 'bob-1',
      toName: 'Bob',
      by: 'Alice',
      reason: null,
      newTaskId: 'task-2',
      reassignedAt: 1,
    }
    handleNandiEvent({ event: 'task://reassigned', payload: reassigned })
    expect(notify).toHaveBeenCalledWith('Reassigned to Bob by Alice')
    vi.mocked(notify).mockClear()
    handleNandiEvent({
      event: 'task://reassigned',
      payload: { ...reassigned, newTaskId: null },
    })
    expect(notify).not.toHaveBeenCalled()
  })
})
