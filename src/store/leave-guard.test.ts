import { beforeEach, describe, expect, it } from 'vitest'
import { useNavigationStore } from './navigation-store'

const state = () => useNavigationStore.getState()

describe('leave guard', () => {
  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
  })

  it('holds a move away from unsaved changes until confirmed', () => {
    state().navigate({ kind: 'workflow', workflowId: 'daily-report' })
    state().setLeaveGuarded(true)

    state().navigate({ kind: 'home' })
    expect(state().current).toEqual({
      kind: 'workflow',
      workflowId: 'daily-report',
    })
    expect(state().pendingLeave).toEqual({
      mode: 'navigate',
      view: { kind: 'home' },
    })

    state().confirmLeave()
    expect(state().current).toEqual({ kind: 'home' })
    expect(state().leaveGuarded).toBe(false)
    expect(state().pendingLeave).toBeNull()
  })

  it('stays put on cancel, and holds Back too', () => {
    state().navigate({ kind: 'trigger', triggerId: 'orders' })
    state().setLeaveGuarded(true)

    state().goBack()
    expect(state().pendingLeave).toEqual({ mode: 'back' })
    state().cancelLeave()
    expect(state().pendingLeave).toBeNull()
    expect(state().current).toEqual({ kind: 'trigger', triggerId: 'orders' })
    // The changes are still there, so it stays guarded
    expect(state().leaveGuarded).toBe(true)
  })

  it('lets a move to the same view through', () => {
    state().navigate({ kind: 'trigger', triggerId: 'orders' })
    state().setLeaveGuarded(true)
    state().navigate({ kind: 'trigger', triggerId: 'orders' })
    expect(state().pendingLeave).toBeNull()
  })
})
