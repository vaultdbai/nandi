import { describe, it, expect } from 'vitest'
import {
  reduceConversation,
  reduceDashboard,
  reducePendingApprovals,
  upsertRun,
  type ConversationEvent,
  type ConversationState,
} from './store-reducers'
import {
  T0,
  agentMessage,
  approvalMessage,
  dashboardSnapshot,
  run,
  userMessage,
} from '@/test/fixtures'

const ids = { agentId: 'writer', conversationId: 'agent:writer' }
const empty: ConversationState = {
  conversationId: 'agent:writer',
  messages: [],
  currentRun: null,
}

const token = (
  messageId: string,
  offset: number,
  text: string
): ConversationEvent => ({
  type: 'token',
  payload: { ...ids, runId: 'r1', messageId, offset, text, createdAt: T0 },
})

/** Applies events in order, failing on any resync */
const apply = (events: ConversationEvent[], start = empty) =>
  events.reduce((state, event) => {
    const result = reduceConversation(state, event)
    expect(result.resync).toBe(false)
    return result.conversation
  }, start)

describe('reduceConversation', () => {
  it('builds agent text from offset-addressed tokens', () => {
    const state = apply([token('m1', 0, 'Hello '), token('m1', 6, 'world')])
    expect(state.messages).toEqual([agentMessage('m1', 'Hello world')])
  })

  it('ignores duplicate tokens', () => {
    const once = apply([token('m1', 0, 'Hello '), token('m1', 6, 'world')])
    const twice = apply(
      [token('m1', 6, 'world'), token('m1', 0, 'Hello ')],
      once
    )
    expect(twice).toEqual(once)
  })

  it('asks for a resync when a token chunk was missed', () => {
    const state = apply([token('m1', 0, 'Hello ')])
    expect(reduceConversation(state, token('m1', 20, 'late')).resync).toBe(true)
    // A non-initial chunk for an unknown message also means we missed some
    expect(reduceConversation(empty, token('m2', 4, 'x')).resync).toBe(true)
  })

  it('upserts messages by id, replacing optimistic copies', () => {
    const pending = { ...userMessage('u1', 'hi', ''), pending: true }
    const state = apply(
      [
        {
          type: 'message',
          payload: { ...ids, message: userMessage('u1', 'hi') },
        },
      ],
      { ...empty, messages: [pending] }
    )
    expect(state.messages).toEqual([userMessage('u1', 'hi')])
  })

  it('applies tool results to their call and resyncs for unknown calls', () => {
    const call: ConversationEvent = {
      type: 'toolCall',
      payload: {
        ...ids,
        runId: 'r1',
        message: {
          kind: 'toolCall',
          id: 'c1',
          runId: 'r1',
          createdAt: T0,
          tool: 'list_dir',
          args: {},
          status: 'running',
          result: null,
          isError: false,
        },
      },
    }
    const result: ConversationEvent = {
      type: 'toolResult',
      payload: {
        ...ids,
        runId: 'r1',
        messageId: 'c1',
        result: 'ok',
        isError: false,
      },
    }
    const state = apply([call, result, result])
    expect(state.messages[0]).toMatchObject({ status: 'done', result: 'ok' })
    expect(reduceConversation(empty, result).resync).toBe(true)
  })

  it('resolves approvals and tracks the current run', () => {
    const state = apply([
      { type: 'run', payload: { ...ids, run: run('r1', 'writer') } },
      {
        type: 'approvalRequest',
        payload: {
          ...ids,
          runId: 'r1',
          approvalId: 'a1',
          message: approvalMessage('a1'),
        },
      },
      {
        type: 'run',
        payload: {
          ...ids,
          run: run('r1', 'writer', { status: 'awaitingApproval' }),
        },
      },
      {
        type: 'approvalResolved',
        payload: { ...ids, runId: 'r1', approvalId: 'a1', status: 'approved' },
      },
    ])
    expect(state.currentRun?.status).toBe('awaitingApproval')
    expect(state.messages[0]).toMatchObject({
      kind: 'approval',
      status: 'approved',
    })

    const done = apply(
      [
        {
          type: 'turnDone',
          payload: {
            ...ids,
            runId: 'r1',
            status: 'completed',
            userMessageId: 'u1',
            finalMessageId: null,
            error: null,
          },
        },
      ],
      state
    )
    expect(done.currentRun).toBeNull()
  })

  it('ignores run updates for other runs once finished', () => {
    const state = apply([
      { type: 'run', payload: { ...ids, run: run('r2', 'writer') } },
    ])
    const finishedOther = apply(
      [
        {
          type: 'run',
          payload: { ...ids, run: run('r1', 'writer', { status: 'failed' }) },
        },
      ],
      state
    )
    expect(finishedOther.currentRun?.id).toBe('r2')
  })
})

describe('reducePendingApprovals', () => {
  it('adds once and removes on resolution', () => {
    const request: ConversationEvent = {
      type: 'approvalRequest',
      payload: {
        ...ids,
        runId: 'r1',
        approvalId: 'a1',
        message: approvalMessage('a1'),
      },
    }
    const added = reducePendingApprovals(
      reducePendingApprovals([], request),
      request
    )
    expect(added).toHaveLength(1)
    expect(added[0]).toMatchObject({ id: 'a1', tool: 'shell' })

    const resolved = reducePendingApprovals(added, {
      type: 'approvalResolved',
      payload: { ...ids, runId: 'r1', approvalId: 'a1', status: 'rejected' },
    })
    expect(resolved).toEqual([])
  })
})

describe('reduceDashboard', () => {
  const item = {
    id: 'a1',
    kind: 'approval' as const,
    agentId: 'writer',
    agentName: 'Writer',
    runId: 'r1',
    messageId: 'a1',
    detail: 'shell: {}',
    createdAt: T0,
  }

  it('upserts and removes attention items, updating the count', () => {
    let dashboard = reduceDashboard(dashboardSnapshot(), {
      type: 'attention',
      payload: { op: 'upsert', item },
    })
    dashboard = reduceDashboard(dashboard, {
      type: 'attention',
      payload: { op: 'upsert', item },
    })
    expect(dashboard.attentionItems).toHaveLength(1)
    expect(dashboard.stats.needsAttention).toBe(1)

    dashboard = reduceDashboard(dashboard, {
      type: 'attention',
      payload: { op: 'remove', itemId: 'a1' },
    })
    expect(dashboard.attentionItems).toEqual([])
    expect(dashboard.stats.needsAttention).toBe(0)
  })

  it('dedupes activity by id, newest first', () => {
    const activity = (id: string, createdAt: number) => ({
      id,
      agentId: 'writer',
      agentName: 'Writer',
      runId: 'r1',
      action: 'started' as const,
      subject: 'x',
      createdAt,
    })
    let dashboard = dashboardSnapshot()
    for (const event of [
      activity('a', T0),
      activity('b', T0 + 1),
      activity('a', T0),
    ]) {
      dashboard = reduceDashboard(dashboard, {
        type: 'activity',
        payload: { item: event },
      })
    }
    expect(dashboard.recentActivity.map(a => a.id)).toEqual(['b', 'a'])
  })
})

describe('upsertRun', () => {
  it('replaces by id and keeps newest first', () => {
    const runs = upsertRun(
      [
        run('r1', 'w', { startedAt: T0 }),
        run('r0', 'w', { startedAt: T0 - 10 }),
      ],
      run('r1', 'w', { startedAt: T0, status: 'completed' })
    )
    expect(runs.map(r => [r.id, r.status])).toEqual([
      ['r1', 'completed'],
      ['r0', 'running'],
    ])
  })
})
