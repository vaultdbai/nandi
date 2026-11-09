import { describe, expect, it } from 'vitest'
import type { WorkItem } from '@/lib/bindings'
import { run } from '@/test/fixtures'
import { activityRows, delegationRow } from './activity-model'
import {
  buildHomeFeed,
  isStale,
  MAX_RECENT,
  STALE_AFTER_MS,
  workRow,
} from './home-model'

const NOW = new Date(2026, 9, 18, 15, 0).getTime()
const MIN = 60_000
const names = (id: string) => id

function item(overrides: Partial<WorkItem>): WorkItem {
  return {
    id: 'w1',
    kind: 'workflow',
    title: 'Daily digest',
    state: 'active',
    owner: null,
    progress: { completedSteps: 1, totalSteps: 4, percent: 25 },
    nextAction: null,
    workflowId: 'digest',
    rating: null,
    startedAt: NOW - 10 * MIN,
    updatedAt: NOW - MIN,
    ...overrides,
  }
}

describe('workRow', () => {
  it('shows a workflow with its step count and retries a failed one', () => {
    expect(workRow(item({}))).toMatchObject({
      status: 'running',
      source: { kind: 'workflow' },
      progress: { done: 1, total: 4 },
      view: { kind: 'work-item', id: 'w1' },
      retry: null,
    })
    expect(workRow(item({ state: 'failed' })).retry).toEqual({
      kind: 'workflow',
      workflowId: 'digest',
    })
  })

  it('rates a finished delegation and retries a failed one as a new attempt', () => {
    const done = workRow(
      item({ id: 't1', kind: 'task', state: 'done', owner: 'Priya' })
    )
    expect(done.source).toEqual({ kind: 'to', name: 'Priya' })
    expect(done.rating).toEqual({ workId: 't1', value: null })
    expect(done.view).toEqual({ kind: 'request', requestId: 't1' })
    expect(
      workRow(item({ id: 't1', kind: 'task', state: 'failed' })).retry
    ).toEqual({ kind: 'request', requestId: 't1' })
  })
})

describe('buildHomeFeed', () => {
  it("lists running work oldest first, without what's in the Inbox", () => {
    const feed = buildHomeFeed({
      now: NOW,
      work: [
        item({ id: 'later', startedAt: NOW - MIN }),
        item({ id: 'first', startedAt: NOW - 5 * MIN }),
        item({
          id: 'approve',
          state: 'waiting',
          nextAction: {
            description: '',
            kind: 'approval',
            deadline: null,
          } as WorkItem['nextAction'],
        }),
      ],
      activity: activityRows(
        [
          run('r1', 'writer', { status: 'running', startedAt: NOW - 3 * MIN }),
          // Waiting on an approval: the Inbox has it
          run('r2', 'writer', { status: 'awaitingApproval' }),
          // Started by a workflow: shown as the workflow
          run('r3', 'writer', { status: 'running', sourceType: 'workflow' }),
        ],
        [],
        names
      ),
    })
    expect(feed.running.map(r => r.id)).toEqual([
      'work:first',
      'run:r1',
      'work:later',
    ])
  })

  it('shows at most 8 recent results from the last day, newest first', () => {
    const runs = Array.from({ length: 12 }, (_, i) =>
      run(`r${i}`, 'writer', {
        status: 'completed',
        startedAt: NOW - (i + 1) * MIN,
        endedAt: NOW - i * MIN,
      })
    )
    const old = run('old', 'writer', {
      status: 'completed',
      startedAt: NOW - 30 * 60 * MIN,
      endedAt: NOW - 26 * 60 * MIN,
    })
    const feed = buildHomeFeed({
      now: NOW,
      work: [],
      activity: activityRows([...runs, old], [], names),
    })
    expect(feed.recent).toHaveLength(MAX_RECENT)
    expect(feed.recent[0]?.id).toBe('run:r0')
    expect(feed.recent.some(r => r.id === 'run:old')).toBe(false)
  })

  it('shows a delegation once: its work item wins over its history row', () => {
    const feed = buildHomeFeed({
      now: NOW,
      work: [item({ id: 't1', kind: 'task', state: 'done', updatedAt: NOW })],
      activity: [
        delegationRow({
          taskId: 't1',
          providerAgent: 'p/a',
          output: null,
          status: 'completed',
          durationMs: 1,
          tokensIn: 0,
          tokensOut: 0,
          error: null,
          direction: 'outbound',
          capability: 'c',
          requesterAgent: 'me/w',
          providerOwner: 'P',
          requesterOwner: null,
          input: 'x',
          startedAt: NOW - MIN,
          endedAt: NOW,
        }),
      ],
    })
    expect(feed.recent.map(r => r.id)).toEqual(['work:t1'])
  })

  it('flags a running row with no update for over 10 minutes', () => {
    const row = workRow(item({ updatedAt: NOW - STALE_AFTER_MS - 1 }))
    expect(isStale(row, NOW)).toBe(true)
    expect(isStale(workRow(item({})), NOW)).toBe(false)
  })
})
