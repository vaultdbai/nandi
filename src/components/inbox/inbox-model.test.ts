import { describe, expect, it } from 'vitest'
import type {
  GatewayAssignment,
  GatewayHold,
  PendingApproval,
  PendingClarification,
  PendingManualStep,
  RequestReturn,
  TrustRequest,
  WorkItem,
} from '@/lib/bindings'
import { buildInbox, inboxBadgeCount, type InboxSources } from './inbox-model'

const NOW = 1_000_000_000

const approval = (
  id: string,
  createdAt: number,
  expiresAt: number | null
): PendingApproval => ({
  id,
  agentId: 'writer',
  runId: `run-${id}`,
  conversationId: 'agent:writer',
  tool: 'shell',
  args: {},
  createdAt,
  expiresAt,
})

const work = (overrides: Partial<WorkItem>): WorkItem => ({
  id: 'w',
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

const clarification = (
  approvalId: string | null,
  deadline: number | null
): PendingClarification => ({
  taskId: 't1',
  agentId: 'writer',
  question: {
    id: 'q1',
    question: 'Which quarter?',
    topic: '',
    requiresHuman: true,
  },
  deadline,
  approvalId,
})

const manual: PendingManualStep = {
  runId: 'run-m',
  workflowId: 'onboarding',
  workflowName: 'Onboarding',
  stepId: 'laptop',
  instructions: 'Tag the laptop',
  deadline: null,
  readyAt: NOW - 50_000,
}

const sources = (overrides: Partial<InboxSources>): InboxSources => ({
  now: NOW,
  noModel: false,
  approvals: [],
  clarifications: [],
  manualSteps: [],
  holds: [],
  assignments: [],
  trustRequests: [],
  work: [],
  dismissed: [],
  returns: [],
  agentName: id => (id === 'writer' ? 'Writer' : id),
  ...overrides,
})

const trustRequest = (instanceId: string, firstAt: number): TrustRequest => ({
  instanceId,
  memberName: 'Sam K.',
  handle: 'sam',
  agentId: 'papers',
  agentName: 'Papers',
  capability: 'summarize-paper',
  attempts: 2,
  firstAt,
  lastAt: firstAt + 1000,
  lastTaskId: 't1',
})

describe('buildInbox: trust requests (task 12)', () => {
  it('lists people waiting to be trusted, oldest first, and counts them', () => {
    const items = buildInbox(
      sources({
        trustRequests: [
          trustRequest('desktop-new', NOW - 1_000),
          trustRequest('desktop-old', NOW - 90_000),
        ],
      })
    )
    expect(items.map(i => i.id)).toEqual([
      'trust-desktop-old',
      'trust-desktop-new',
    ])
    expect(inboxBadgeCount(items)).toBe(2)
  })
})

const hold = (
  taskId: string,
  expiresAt: number,
  status: GatewayHold['status'] = 'pending'
): GatewayHold => ({
  taskId,
  callerInstance: 'desktop-sam',
  callerName: 'Sam Lee',
  capability: 'finance.risk',
  summary: 'Summarize Q3 risk',
  status,
  createdAt: NOW - 60_000,
  expiresAt,
})

describe('buildInbox: held requests (task 5)', () => {
  it('lists pending holds by their expiry, and counts them', () => {
    const items = buildInbox(
      sources({
        holds: [hold('late', NOW + 50_000), hold('soon', NOW + 10_000)],
      })
    )
    expect(items.map(i => i.id)).toEqual(['hold-soon', 'hold-late'])
    expect(inboxBadgeCount(items)).toBe(2)
  })

  it('leaves out decided and expired holds', () => {
    const items = buildInbox(
      sources({
        holds: [
          hold('expired', NOW - 1),
          hold('approved', NOW + 10_000, 'approved'),
        ],
      })
    )
    expect(items).toEqual([])
  })
})

const assignment = (
  taskId: string,
  dueAt: number | null,
  status: GatewayAssignment['status'] = 'open'
): GatewayAssignment => ({
  taskId,
  callerInstance: 'desktop-sam',
  callerName: 'Sam Lee',
  request: 'Upload the SOC 1 evidence',
  status,
  note: null,
  createdAt: NOW - 60_000,
  dueAt,
  closedAt: null,
  links: [],
})

describe('buildInbox: assignments', () => {
  it('lists open assignments soonest due first, overdue ones included', () => {
    const items = buildInbox(
      sources({
        assignments: [
          assignment('later', NOW + 50_000),
          assignment('overdue', NOW - 10_000),
          assignment('undated', null),
          assignment('closed', NOW + 1, 'done'),
        ],
        holds: [hold('soon', NOW + 10_000)],
      })
    )
    expect(items.map(i => i.id)).toEqual([
      'assignment-overdue',
      'hold-soon',
      'assignment-later',
      'assignment-undated',
    ])
    expect(inboxBadgeCount(items)).toBe(4)
  })
})

describe('buildInbox', () => {
  it('is empty when nothing waits on you', () => {
    expect(buildInbox(sources({}))).toEqual([])
  })

  it('orders by deadline first, then age, failures last', () => {
    const items = buildInbox(
      sources({
        noModel: true,
        approvals: [
          approval('old', NOW - 90_000, NOW + 600_000),
          approval('soon', NOW - 10_000, NOW + 60_000),
          approval('open', NOW - 99_000, null),
        ],
        manualSteps: [manual],
        work: [
          work({ id: 'f1', state: 'failed', updatedAt: NOW - 5_000 }),
          work({
            id: 'wa',
            state: 'waiting',
            updatedAt: NOW - 20_000,
            nextAction: { description: '', kind: 'approval', deadline: null },
          }),
        ],
      })
    )
    expect(items.map(item => item.id)).toEqual([
      'approval-soon',
      'approval-old',
      'no-model',
      'approval-open',
      'manual-run-m-laptop',
      'work-wa',
      'failed-f1',
    ])
  })

  it('names the agent asking', () => {
    const [item] = buildInbox(
      sources({ approvals: [approval('a', NOW, NOW + 1000)] })
    )
    expect(item?.kind === 'approval' && item.agentName).toBe('Writer')
  })

  it('drops approvals past their deadline (they are expiring)', () => {
    expect(
      buildInbox(sources({ approvals: [approval('gone', 0, NOW)] }))
    ).toEqual([])
  })

  it('shows only questions escalated to a person, and not expired ones', () => {
    const items = buildInbox(
      sources({
        clarifications: [
          clarification(null, NOW + 1000),
          clarification('ap', NOW + 1000),
          { ...clarification('ap2', NOW - 1), taskId: 't2' },
        ],
      })
    )
    expect(items).toHaveLength(1)
    expect(items[0]?.kind).toBe('clarification')
  })

  it('shows a run waiting on a manual step once, as the step', () => {
    const items = buildInbox(
      sources({
        manualSteps: [manual],
        work: [
          work({
            id: 'run-m',
            state: 'waiting',
            nextAction: { description: '', kind: 'approval', deadline: null },
          }),
        ],
      })
    )
    expect(items.map(item => item.kind)).toEqual(['manualStep'])
  })

  it('offers failures from the last day that were not dismissed', () => {
    const items = buildInbox(
      sources({
        work: [
          work({ id: 'recent', state: 'failed', updatedAt: NOW - 1000 }),
          work({
            id: 'stale',
            state: 'failed',
            updatedAt: NOW - 2 * 86_400_000,
          }),
          work({ id: 'hidden', state: 'failed', updatedAt: NOW - 1000 }),
          work({ id: 'fine', state: 'done' }),
        ],
        dismissed: ['hidden'],
      })
    )
    expect(items.map(item => item.id)).toEqual(['failed-recent'])
  })
})

describe('buildInbox: requests that came back', () => {
  const ret = (
    taskId: string,
    outcome: RequestReturn['outcome'],
    returnedAt: number
  ): RequestReturn => ({
    taskId,
    fromName: 'Lena Fischer',
    providerAgent: 'lena/gateway',
    capability: 'ask-me',
    outcome,
    summary: 'Nov 12',
    request: 'When is the walkthrough?',
    returnedAt,
  })

  it('offers each one after what waits on you, outside the badge', () => {
    const items = buildInbox(
      sources({
        approvals: [approval('a1', NOW - 5000, NOW + 60_000)],
        returns: [ret('t1', 'answered', NOW - 1000)],
      })
    )
    expect(items.map(item => item.id)).toEqual(['approval-a1', 'return-t1'])
    expect(inboxBadgeCount(items)).toBe(1)
  })

  it('shows a request that came back failed once, as the return', () => {
    const items = buildInbox(
      sources({
        work: [work({ id: 't2', state: 'failed', updatedAt: NOW - 1000 })],
        returns: [ret('t2', 'expired', NOW - 1000)],
      })
    )
    expect(items.map(item => item.id)).toEqual(['return-t2'])
  })
})

describe('inboxBadgeCount', () => {
  it('counts what waits on you, not failures', () => {
    const items = buildInbox(
      sources({
        noModel: true,
        approvals: [approval('a', NOW, NOW + 1000)],
        work: [work({ id: 'f', state: 'failed', updatedAt: NOW })],
      })
    )
    expect(items).toHaveLength(3)
    expect(inboxBadgeCount(items)).toBe(2)
  })
})
