import { describe, expect, it } from 'vitest'
import type { NextRun } from '@/lib/bindings'
import { nextRunOfTarget, nextRunOfTrigger } from './next-run-model'

const run = (triggerId: string, targetId: string, at: number): NextRun => ({
  triggerId,
  targetType: 'workflow',
  targetId,
  at,
})

describe('next runs', () => {
  const runs = [
    run('a', 'bills', 300),
    run('b', 'bills', 100),
    run('c', 'trip', 200),
  ]

  it('is the soonest of the triggers that start a workflow', () => {
    expect(nextRunOfTarget(runs, 'workflow', 'bills')).toBe(100)
    expect(nextRunOfTarget(runs, 'agent', 'bills')).toBeNull()
    expect(nextRunOfTarget(runs, 'workflow', 'none')).toBeNull()
  })

  it('is a trigger own next time', () => {
    expect(nextRunOfTrigger(runs, 'c')).toBe(200)
    expect(nextRunOfTrigger(runs, 'hook')).toBeNull()
  })
})
