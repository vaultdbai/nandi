import { describe, expect, it } from 'vitest'
import type { AgentCard, TaskResult, TrustRule } from '@/lib/bindings'
import {
  agentIdOf,
  callersOf,
  counterpart,
  errorMessage,
  filterHistory,
  historyFacets,
  ownCard,
  providersByCapability,
  upsertRow,
} from './useDelegation'

function taskRow(overrides: Partial<TaskResult> = {}): TaskResult {
  return {
    taskId: 't1',
    providerAgent: 'them/papers',
    output: null,
    status: 'completed',
    durationMs: 1200,
    tokensIn: 10,
    tokensOut: 20,
    error: null,
    direction: 'outbound',
    capability: 'summarize',
    requesterAgent: 'me/writer',
    providerOwner: 'alice',
    requesterOwner: 'me',
    input: { text: 'hi' },
    startedAt: 1_000,
    endedAt: 2_200,
    ...overrides,
  }
}

function card(id: string, capabilities: string[]): AgentCard {
  return {
    id,
    name: id,
    description: '',
    instanceId: id.split('/')[0] ?? '',
    endpoint: '',
    capabilities: capabilities.map(name => ({ name })),
    acceptsDelegation: true,
    maxConcurrent: 2,
    currentLoad: 0,
    owner: { githubHandle: '', displayName: '' },
    signature: null,
    issuedAt: 0,
  }
}

describe('delegation helpers', () => {
  it('reads the agent part of a card ID', () => {
    expect(agentIdOf('inst/papers')).toBe('papers')
    expect(agentIdOf('papers')).toBe('papers')
  })

  it('takes the other side of a row by direction', () => {
    expect(counterpart(taskRow())).toEqual({
      cardId: 'them/papers',
      owner: 'alice',
    })
    expect(counterpart(taskRow({ direction: 'inbound' }))).toEqual({
      cardId: 'me/writer',
      owner: 'me',
    })
  })

  it('filters by status, capability, and owner', () => {
    const rows = [
      taskRow({ taskId: 'a' }),
      taskRow({ taskId: 'b', status: 'failed', capability: 'translate' }),
      taskRow({ taskId: 'c', providerOwner: 'bob' }),
    ]
    const ids = (filtered: TaskResult[]) => filtered.map(r => r.taskId)
    expect(ids(filterHistory(rows, {}))).toEqual(['a', 'b', 'c'])
    expect(ids(filterHistory(rows, { status: 'failed' }))).toEqual(['b'])
    expect(ids(filterHistory(rows, { capability: 'summarize' }))).toEqual([
      'a',
      'c',
    ])
    expect(ids(filterHistory(rows, { owner: 'bob' }))).toEqual(['c'])
    expect(historyFacets(rows)).toEqual({
      capabilities: ['summarize', 'translate'],
      owners: ['alice', 'bob'],
    })
  })

  it('groups providers by capability, leaving the agent out', () => {
    const cards = [
      card('me/writer', ['write']),
      card('them/papers', ['summarize', 'find']),
      card('other/x', ['summarize']),
    ]
    const groups = providersByCapability(cards, 'writer')
    expect(groups.map(([name, list]) => [name, list.map(c => c.id)])).toEqual([
      ['find', ['them/papers']],
      ['summarize', ['them/papers', 'other/x']],
    ])
    expect(ownCard(cards, 'writer')?.id).toBe('me/writer')
  })

  it('lists callers from allow rules and past inbound tasks', () => {
    const rules: TrustRule[] = [
      {
        requester: '*',
        providerAgent: 'papers',
        policy: 'allow',
        updatedAt: 0,
      },
      {
        requester: 'x/y',
        providerAgent: 'papers',
        policy: 'deny',
        updatedAt: 0,
      },
      {
        requester: 'z/w',
        providerAgent: 'other',
        policy: 'allow',
        updatedAt: 0,
      },
    ]
    const history = [
      taskRow({
        direction: 'inbound',
        providerAgent: 'me/papers',
        requesterAgent: 'them/writer',
      }),
      taskRow({
        direction: 'inbound',
        status: 'failed',
        requesterAgent: 'q/q',
      }),
    ]
    expect(callersOf('papers', rules, history)).toEqual(['*', 'them/writer'])
  })

  it('puts updated rows first without duplicates', () => {
    const rows = [taskRow({ taskId: 'a' }), taskRow({ taskId: 'b' })]
    const updated = upsertRow(rows, taskRow({ taskId: 'b', status: 'failed' }))
    expect(updated.map(r => [r.taskId, r.status])).toEqual([
      ['b', 'failed'],
      ['a', 'completed'],
    ])
  })

  it('reads error messages from any thrown shape', () => {
    expect(errorMessage('plain')).toBe('plain')
    expect(errorMessage(new Error('boom'))).toBe('boom')
    expect(errorMessage({ status: 'error', error: 'nope' })).toBe('nope')
  })
})
