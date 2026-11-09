import { describe, expect, it } from 'vitest'
import type { HomeRow } from '@/components/home/home-model'
import type { InboxItem } from '@/components/inbox/inbox-model'
import type { WorkItem } from '@/lib/bindings'
import { buildTray, homeTrayRow, inboxTrayRow, TRAY_LIMITS } from './tray-model'

const NOW = 1_000_000_000

const base = (id: string) => ({ id, createdAt: NOW, deadline: null })

const work = (overrides: Partial<WorkItem> = {}): WorkItem => ({
  id: 'w1',
  kind: 'workflow',
  title: 'Digest',
  state: 'active',
  owner: null,
  progress: { completedSteps: 0, totalSteps: 1, percent: 0 },
  nextAction: null,
  workflowId: 'digest',
  rating: null,
  startedAt: NOW - 1000,
  updatedAt: NOW - 1000,
  ...overrides,
})

const approval: InboxItem = {
  ...base('approval:a1'),
  kind: 'approval',
  agentName: 'Writer',
  approval: {
    id: 'a1',
    agentId: 'writer',
    runId: 'run-a1',
    conversationId: 'agent:writer',
    tool: 'shell',
    args: {},
    createdAt: NOW,
    expiresAt: null,
  },
}

const homeRow = (overrides: Partial<HomeRow> = {}): HomeRow => ({
  id: 'work:w1',
  title: 'Weekly digest',
  status: 'running',
  source: { kind: 'workflow' },
  agent: null,
  progress: null,
  startedAt: NOW - 1000,
  endedAt: null,
  updatedAt: NOW - 1000,
  view: { kind: 'work-item', id: 'w1' },
  rating: null,
  retry: null,
  ...overrides,
})

const noModel: InboxItem = { ...base('noModel'), kind: 'noModel' }

describe('inboxTrayRow', () => {
  it('lets a tool approval be decided in place, and opens its chat', () => {
    const row = inboxTrayRow(approval)
    expect(row.tone).toBe('needsYou')
    expect(row.label).toEqual({
      key: 'tray.approval',
      values: { tool: 'shell' },
    })
    expect(row.meta).toEqual({ text: 'Writer' })
    expect(row.view).toEqual({
      kind: 'inbox-thread',
      conversationId: 'inbox:approval:a1',
    })
    expect(row.approval).toEqual({ agentId: 'writer', approvalId: 'a1' })
  })

  it('sends a missing model to Settings › Models', () => {
    expect(inboxTrayRow(noModel).view).toEqual({
      kind: 'settings',
      section: 'models',
    })
  })

  it('opens a workflow approval on its run', () => {
    const row = inboxTrayRow({
      ...base('wa'),
      kind: 'workflowApproval',
      item: work(),
    })
    expect(row.view).toEqual({ kind: 'work-item', id: 'w1' })
    expect(row.approval).toBeNull()
  })

  it('shows a question as itself, opening its chat', () => {
    const row = inboxTrayRow({
      ...base('q'),
      kind: 'clarification',
      clarification: {
        taskId: 't1',
        agentId: 'writer',
        question: {
          id: 'q1',
          question: 'Which quarter?',
          topic: '',
          requiresHuman: true,
        },
        deadline: null,
        approvalId: null,
      },
    })
    expect(row.label).toEqual({ text: 'Which quarter?' })
    expect(row.view).toEqual({
      kind: 'inbox-thread',
      conversationId: 'inbox:clarification:t1/q1',
    })
  })

  it('colours a failure as failed, and opens its chat', () => {
    const row = inboxTrayRow({
      ...base('f'),
      kind: 'failed',
      item: work({ kind: 'request', state: 'failed', title: 'Report' }),
    })
    expect(row.tone).toBe('failed')
    expect(row.view).toEqual({
      kind: 'inbox-thread',
      conversationId: 'inbox:failed:w1',
    })
  })

  it('opens a manual step in its chat, where its card finishes it', () => {
    const row = inboxTrayRow({
      ...base('m'),
      kind: 'manualStep',
      step: {
        runId: 'r1',
        workflowId: 'digest',
        workflowName: 'Digest',
        stepId: 's1',
        instructions: 'Sign it',
        deadline: null,
        readyAt: NOW,
      },
    })
    expect(row.view).toEqual({
      kind: 'inbox-thread',
      conversationId: 'inbox:manualStep:r1/s1',
    })
  })

  it('opens queued work from outside in its intake thread', () => {
    const row = inboxTrayRow({
      ...base('i'),
      kind: 'intake',
      work: {
        id: 'q1',
        at: NOW,
        source: 'email',
        action: 'start',
        requestId: null,
        requestTitle: null,
        text: 'Pay the Acme bill',
        sender: null,
        screening: { verdict: 'clean', reasons: [], links: [] },
        status: 'pending',
        resultRequestId: null,
        threadId: 'intake:q1',
        plan: null,
      },
    })
    expect(row.label).toEqual({
      key: 'tray.intake',
      values: { text: 'Pay the Acme bill' },
    })
    expect(row.view).toEqual({
      kind: 'intake-thread',
      conversationId: 'intake:q1',
    })
  })
})

describe('homeTrayRow', () => {
  it('colours by status', () => {
    expect(homeTrayRow(homeRow()).tone).toBe('working')
    expect(homeTrayRow(homeRow({ status: 'done' })).tone).toBe('done')
    expect(homeTrayRow(homeRow({ status: 'failed' })).tone).toBe('failed')
    expect(homeTrayRow(homeRow({ status: 'cancelled' })).tone).toBe('quiet')
  })

  it('notes the step it is on (one-based, never past the last)', () => {
    expect(
      homeTrayRow(homeRow({ progress: { done: 1, total: 3 } })).meta
    ).toEqual({ key: 'tray.step', values: { done: 2, total: 3 } })
    expect(
      homeTrayRow(homeRow({ progress: { done: 3, total: 3 } })).meta
    ).toEqual({ key: 'tray.step', values: { done: 3, total: 3 } })
  })

  it('names the agent when there are no steps', () => {
    expect(homeTrayRow(homeRow({ agent: 'Writer' })).meta).toEqual({
      text: 'Writer',
    })
    expect(homeTrayRow(homeRow()).meta).toBeNull()
  })
})

describe('buildTray', () => {
  it('has Needs you, Working and Done, each with its All link', () => {
    const groups = buildTray({ inbox: [], running: [], recent: [] })
    expect(groups.map(group => group.id)).toEqual([
      'needsYou',
      'working',
      'done',
    ])
    expect(groups.map(group => group.all)).toEqual([
      { kind: 'inbox' },
      { kind: 'home-activity', filters: { status: 'running' } },
      { kind: 'home-activity' },
    ])
  })

  it('shows a few rows per group but counts them all', () => {
    const running = Array.from({ length: TRAY_LIMITS.working + 3 }, (_, i) =>
      homeRow({ id: `r${i}` })
    )
    const [, working] = buildTray({ inbox: [approval], running, recent: [] })
    expect(working?.total).toBe(TRAY_LIMITS.working + 3)
    expect(working?.rows).toHaveLength(TRAY_LIMITS.working)
    expect(working?.rows[0]?.id).toBe('r0')
  })

  it('keeps the Inbox order', () => {
    const [needsYou] = buildTray({
      inbox: [noModel, approval],
      running: [],
      recent: [],
    })
    expect(needsYou?.rows.map(row => row.id)).toEqual([
      'noModel',
      'approval:a1',
    ])
  })
})
