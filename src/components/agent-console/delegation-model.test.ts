import { describe, expect, it } from 'vitest'
import type { RunSummary } from '@/lib/bindings'
import type { UiMessage } from '@/types/agent'
import {
  delegationState,
  delegationWindows,
  isDelegation,
  runsOfCall,
  splitChildMessages,
} from './delegation-model'

const call = (
  id: string,
  createdAt: number,
  tool = 'delegate_to_agent',
  runId: string | null = 'p1'
): UiMessage => ({
  kind: 'toolCall',
  id,
  runId,
  createdAt,
  tool,
  args: {},
  status: 'running',
  result: null,
  isError: false,
})

const run = (id: string, startedAt: number): RunSummary => ({
  id,
  agentId: 'researcher',
  conversationId: `task:${id}`,
  status: 'running',
  input: '',
  summary: null,
  startedAt,
  endedAt: null,
  inputTokens: 0,
  outputTokens: 0,
  costUsd: null,
  model: null,
  sourceType: 'agent',
  sourceId: 'me/nandi',
  sourceLabel: null,
  priority: 100,
  queuedAt: null,
  parentRunId: null,
  compactionSummary: null,
})

describe('isDelegation', () => {
  it('is a delegate_to_agent call of a run', () => {
    expect(isDelegation(call('d', 0))).toBe(true)
    expect(isDelegation(call('t', 0, 'fs_read'))).toBe(false)
    expect(isDelegation(call('d', 0, 'delegate_to_agent', null))).toBe(false)
  })
})

describe('delegationWindows and runsOfCall', () => {
  it('gives each call the runs started until the next call', () => {
    const windows = delegationWindows([
      call('d1', 10),
      call('t', 15, 'fs_read'),
      call('d2', 30),
      call('other', 40, 'delegate_to_agent', 'p2'),
    ])
    expect([...windows]).toEqual([
      ['d1', 30],
      ['d2', null],
      ['other', null],
    ])
    const runs = [run('late', 35), run('a', 12), run('b', 20), run('before', 5)]
    expect(runsOfCall(runs, 10, 30).map(r => r.id)).toEqual(['a', 'b'])
    expect(runsOfCall(runs, 30, null).map(r => r.id)).toEqual(['late'])
  })
})

describe('delegationState', () => {
  it('reads like a person: working, needs you, done or stopped', () => {
    expect(delegationState('running')).toBe('working')
    expect(delegationState('queued')).toBe('working')
    expect(delegationState('awaitingApproval')).toBe('needsYou')
    expect(delegationState('completed')).toBe('done')
    expect(delegationState('failed')).toBe('stopped')
    expect(delegationState('cancelled')).toBe('stopped')
  })
})

describe('splitChildMessages', () => {
  it('keeps the waiting approvals out of the folded steps', () => {
    const pending: UiMessage = {
      kind: 'approval',
      id: 'a1',
      runId: 'c1',
      createdAt: 0,
      tool: 'send_email',
      args: {},
      status: 'pending',
      expiresAt: null,
    }
    const decided: UiMessage = { ...pending, id: 'a0', status: 'approved' }
    const step = call('t', 0, 'fs_read', 'c1')
    const split = splitChildMessages([step, decided, pending])
    expect(split.waiting.map(m => m.id)).toEqual(['a1'])
    expect(split.steps.map(m => m.id)).toEqual(['t', 'a0'])
  })
})
