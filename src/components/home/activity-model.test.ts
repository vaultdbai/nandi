import { describe, expect, it } from 'vitest'
import type { TaskResult } from '@/lib/bindings'
import { run } from '@/test/fixtures'
import {
  activityFacets,
  activityRows,
  activitySummary,
  activityToCsv,
  capabilityLabel,
  delegationRow,
  filterActivity,
  firstLine,
  hasActivityFilters,
  runRow,
  startOfDay,
} from './activity-model'

const NOW = new Date(2026, 9, 18, 15, 0).getTime()
const HOUR = 3_600_000
const names = (id: string) => ({ writer: 'Writer', reader: 'Reader' })[id] ?? id

function task(overrides: Partial<TaskResult>): TaskResult {
  return {
    taskId: 't1',
    providerAgent: 'priya/analyst',
    output: null,
    status: 'completed',
    durationMs: 10,
    tokensIn: 5,
    tokensOut: 7,
    error: null,
    direction: 'outbound',
    capability: 'finance.risk-summary',
    requesterAgent: 'me/writer',
    providerOwner: 'Priya',
    requesterOwner: null,
    input: 'Summarize Q3 risk\nwith detail',
    startedAt: NOW - HOUR,
    endedAt: NOW - HOUR / 2,
    ...overrides,
  }
}

describe('rows', () => {
  it('maps a run to its source, state and Run detail', () => {
    const row = runRow(
      run('r1', 'writer', {
        status: 'awaitingApproval',
        sourceType: 'cron',
        input: '\n  Draft the digest \nmore',
        startedAt: NOW,
      }),
      names
    )
    expect(row).toMatchObject({
      id: 'run:r1',
      title: 'Draft the digest',
      status: 'waiting',
      source: 'schedule',
      direction: 'local',
      agent: 'Writer',
      agentId: 'writer',
      view: { kind: 'run-detail', runId: 'r1' },
    })
  })

  it("leaves out Studio's runs and runs a delegation started", () => {
    expect(runRow(run('s', '@studio'), names)).toBeNull()
    expect(
      runRow(run('d', 'writer', { sourceType: 'agent' }), names)
    ).toBeNull()
  })

  it('maps delegations both ways to their Request', () => {
    expect(delegationRow(task({}))).toMatchObject({
      id: 'task:t1',
      title: 'Summarize Q3 risk',
      status: 'done',
      source: 'you',
      direction: 'outgoing',
      member: 'Priya',
      tokens: 12,
      view: { kind: 'request', requestId: 't1' },
    })
    const incoming = delegationRow(
      task({
        direction: 'inbound',
        status: 'timedOut',
        requesterAgent: 'sam/bot',
        requesterOwner: null,
        providerAgent: 'me/analyst',
        input: null,
        error: 'deadline passed',
      })
    )
    expect(incoming).toMatchObject({
      title: 'Finance risk summary',
      status: 'failed',
      source: 'peer',
      direction: 'incoming',
      agent: 'analyst',
      member: 'bot',
      detail: 'deadline passed',
    })
  })

  it('merges runs and delegations newest first', () => {
    const rows = activityRows(
      [run('old', 'writer', { startedAt: NOW - 5 * HOUR })],
      [task({ startedAt: NOW - HOUR })],
      names
    )
    expect(rows.map(r => r.id)).toEqual(['task:t1', 'run:old'])
  })

  it('names capabilities and first lines in plain words', () => {
    expect(capabilityLabel('code-review.python')).toBe('Code review python')
    expect(firstLine('x'.repeat(200)).length).toBe(120)
    expect(firstLine('')).toBe('')
  })
})

describe('filters', () => {
  const rows = activityRows(
    [
      run('today', 'writer', { startedAt: NOW - HOUR, status: 'completed' }),
      run('week', 'reader', {
        startedAt: NOW - 3 * 24 * HOUR,
        status: 'failed',
        sourceType: 'webhook',
      }),
    ],
    [task({ startedAt: NOW - 2 * HOUR })],
    names
  )

  it('defaults to the last 7 days and combines every filter (AND)', () => {
    expect(filterActivity(rows, {}, NOW)).toHaveLength(3)
    expect(filterActivity(rows, { range: 'today' }, NOW)).toHaveLength(2)
    expect(
      filterActivity(rows, { status: 'failed', source: 'trigger' }, NOW).map(
        r => r.id
      )
    ).toEqual(['run:week'])
    expect(
      filterActivity(rows, { status: 'failed', source: 'you' }, NOW)
    ).toHaveLength(0)
    expect(
      filterActivity(rows, { direction: 'outgoing', member: 'Priya' }, NOW)
    ).toHaveLength(1)
  })

  it('filters by agent name or ID, and searches titles and people', () => {
    expect(filterActivity(rows, { agent: 'Writer' }, NOW)).toHaveLength(1)
    expect(filterActivity(rows, { agent: 'writer' }, NOW)).toHaveLength(1)
    expect(filterActivity(rows, { search: 'q3' }, NOW)).toHaveLength(1)
    expect(filterActivity(rows, { search: 'priya' }, NOW)).toHaveLength(1)
  })

  it('knows when anything narrows the list', () => {
    expect(hasActivityFilters({})).toBe(false)
    expect(hasActivityFilters({ range: '7d' })).toBe(false)
    expect(hasActivityFilters({ range: 'today' })).toBe(true)
    expect(hasActivityFilters({ search: '' })).toBe(false)
  })

  it('offers each agent and member once', () => {
    expect(activityFacets(rows)).toEqual({
      agents: ['Reader', 'Writer'],
      members: ['Priya'],
    })
  })

  it('starts the day at local midnight', () => {
    expect(new Date(startOfDay(NOW)).getHours()).toBe(0)
  })
})

describe('summary and CSV', () => {
  const rows = activityRows(
    [
      run('a', 'writer', {
        status: 'completed',
        costUsd: 0.5,
        inputTokens: 10,
      }),
      run('b', 'writer', { status: 'failed', input: '=SUM(A1)' }),
    ],
    [],
    names
  )

  it('totals what the filters show', () => {
    expect(activitySummary(rows)).toMatchObject({
      total: 2,
      done: 1,
      failed: 1,
      costUsd: 0.5,
    })
  })

  it('exports the rows, quoting formulas', () => {
    const csv = activityToCsv(rows)
    const lines = csv.trim().split('\r\n')
    expect(lines[0]).toBe(
      'startedAt,endedAt,title,status,source,direction,agent,member,tokens,costUsd'
    )
    expect(lines).toHaveLength(3)
    expect(csv).toContain("'=SUM(A1)")
  })
})
