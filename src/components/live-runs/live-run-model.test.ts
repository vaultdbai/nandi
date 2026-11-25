import { describe, expect, it } from 'vitest'
import type { RunPlan, RunPlanStep, RunSummary } from '@/lib/bindings'
import type { UiMessage } from '@/types/agent'
import {
  FINISHED_FOR_MS,
  isLiveStatus,
  liveGroup,
  liveOrigin,
  messagesFrom,
  orderLive,
  statusLine,
} from './live-run-model'

const run = (over: Partial<RunSummary> = {}): RunSummary => ({
  id: 'r1',
  agentId: 'writer',
  conversationId: 'agent:writer',
  status: 'running',
  input: 'hi',
  summary: null,
  startedAt: 0,
  endedAt: null,
  inputTokens: 0,
  outputTokens: 0,
  costUsd: null,
  model: null,
  sourceType: 'user',
  sourceId: null,
  sourceLabel: null,
  priority: 100,
  queuedAt: null,
  parentRunId: null,
  compactionSummary: null,
  ...over,
})

const names: Record<string, string> = { nandi: 'Nandi', writer: 'Writer' }
const agentName = (id: string) => names[id]

describe('isLiveStatus', () => {
  it('is running or waiting for an approval', () => {
    expect(isLiveStatus('running')).toBe(true)
    expect(isLiveStatus('awaitingApproval')).toBe(true)
    expect(isLiveStatus('queued')).toBe(false)
    expect(isLiveStatus('completed')).toBe(false)
  })
})

describe('liveGroup', () => {
  it('reads needs you, working or finished from the status', () => {
    expect(liveGroup(run({ status: 'awaitingApproval' }))).toBe('needsYou')
    expect(liveGroup(run({ status: 'running' }))).toBe('working')
    expect(liveGroup(run({ status: 'failed' }))).toBe('finished')
  })
})

describe('orderLive', () => {
  const now = 10 * FINISHED_FOR_MS

  it('puts what needs the person first, then working, then just finished', () => {
    const runs = [
      run({ id: 'old', startedAt: 1 }),
      run({ id: 'new', startedAt: 3 }),
      run({ id: 'waiting', startedAt: 2, status: 'awaitingApproval' }),
      run({ id: 'done', startedAt: 4, status: 'completed', endedAt: now - 5 }),
    ]
    expect(orderLive(runs, now).map(r => r.id)).toEqual([
      'waiting',
      'new',
      'old',
      'done',
    ])
  })

  it('lets a finished run fade out after an hour', () => {
    const runs = [
      run({
        id: 'stale',
        status: 'failed',
        endedAt: now - FINISHED_FOR_MS - 1,
      }),
      run({ id: 'queued', status: 'queued' }),
    ]
    expect(orderLive(runs, now)).toEqual([])
  })
})

describe('statusLine', () => {
  const step = (title: string, status: RunPlanStep['status']): RunPlanStep => ({
    title,
    kind: 'read',
    evidence: 'none',
    note: null,
    status,
    result: null,
    files: [],
  })
  const plan = (steps: RunPlanStep[]): RunPlan => ({
    runId: 'r1',
    revision: 1,
    steps,
    updatedAt: 0,
  })
  const said = (content: string): UiMessage => ({
    kind: 'agent',
    id: content,
    runId: 'r1',
    createdAt: 0,
    content,
  })

  it('is the step it is on, with its number', () => {
    const steps = [
      step('Open the inbox', 'done'),
      step('Read 3 emails from Acme', 'doing'),
      step('Draft replies', 'pending'),
    ]
    expect(statusLine(plan(steps), [])).toEqual({
      text: 'Read 3 emails from Acme',
      step: { number: 2, total: 3 },
    })
  })

  it('is the next step between steps', () => {
    const steps = [step('Open', 'done'), step('Draft', 'pending')]
    expect(statusLine(plan(steps), [])?.text).toBe('Draft')
  })

  it('falls back to the latest words, one short line, never a tool', () => {
    const messages: UiMessage[] = [
      said('Looking now.'),
      {
        kind: 'toolCall',
        id: 't1',
        runId: 'r1',
        createdAt: 0,
        tool: 'fs_read',
        args: {},
        status: 'running',
        result: null,
        isError: false,
      },
    ]
    expect(statusLine(null, messages)).toEqual({ text: 'Looking now.' })
    expect(statusLine(null, [said('First line\nmore')])?.text).toBe(
      'First line'
    )
    expect(statusLine(null, [said('x'.repeat(300))])?.text).toHaveLength(120)
    expect(statusLine(null, [])).toBeNull()
  })
})

describe('liveOrigin', () => {
  it('says how each run started', () => {
    expect(liveOrigin(run(), agentName)).toEqual({ key: 'liveWork.origin.you' })
    expect(
      liveOrigin(
        run({ sourceType: 'workflow', sourceLabel: 'Pay bills · step 1' }),
        agentName
      )
    ).toEqual({
      key: 'liveWork.origin.workflow',
      values: { name: 'Pay bills · step 1' },
    })
    expect(
      liveOrigin(
        run({ sourceType: 'cron', sourceLabel: 'Morning brief' }),
        agentName
      )
    ).toEqual({ text: 'Morning brief' })
    expect(liveOrigin(run({ sourceType: 'webhook' }), agentName)).toEqual({
      key: 'liveWork.origin.trigger',
    })
    expect(liveOrigin(run({ sourceType: 'email' }), agentName)).toEqual({
      key: 'liveWork.origin.email',
    })
    expect(liveOrigin(run({ sourceType: 'calendarCheck' }), agentName)).toEqual(
      { key: 'liveWork.origin.calendar' }
    )
  })

  it('names the agent that asked, local or by card ID', () => {
    expect(
      liveOrigin(run({ sourceType: 'agent', sourceId: 'nandi' }), agentName)
    ).toEqual({ key: 'liveWork.origin.askedBy', values: { name: 'Nandi' } })
    expect(
      liveOrigin(
        run({ sourceType: 'agent', sourceId: 'inst-1/nandi' }),
        agentName
      )
    ).toEqual({ key: 'liveWork.origin.askedBy', values: { name: 'Nandi' } })
    expect(
      liveOrigin(
        run({
          sourceType: 'agent',
          sourceId: 'far/away',
          sourceLabel: 'Delegated by Ada · research',
        }),
        agentName
      )
    ).toEqual({ text: 'Delegated by Ada · research' })
    expect(liveOrigin(run({ sourceType: 'agent' }), agentName)).toEqual({
      key: 'liveWork.origin.anotherAgent',
    })
  })
})

describe('messagesFrom', () => {
  const message = (id: string, runId: string | null): UiMessage => ({
    kind: 'agent',
    id,
    runId,
    createdAt: 0,
    content: id,
  })

  it('starts at the run and keeps the turns after it', () => {
    const messages = [
      message('earlier', 'r0'),
      message('first', 'r1'),
      message('second', 'r1'),
      message('reply', 'r2'),
      message('pending', null),
    ]
    expect(messagesFrom(messages, 'r1').map(m => m.id)).toEqual([
      'first',
      'second',
      'reply',
      'pending',
    ])
  })

  it('is empty until the run has a message', () => {
    expect(messagesFrom([message('earlier', 'r0')], 'r1')).toEqual([])
  })
})
