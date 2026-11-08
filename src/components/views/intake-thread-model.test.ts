import { describe, expect, it } from 'vitest'
import type { IntakeThread, QueuedWork, Request } from '@/lib/bindings'
import {
  canMarkStepsDone,
  threadModel,
  workflowProposalFor,
} from './intake-thread-model'

const NOW = new Date('2026-11-07T12:00:00Z')

function work(overrides: Partial<QueuedWork> = {}): QueuedWork {
  return {
    id: 'q1',
    at: NOW.getTime() - 2 * 60 * 60 * 1000,
    source: 'email',
    action: 'start',
    requestId: null,
    requestTitle: null,
    text: 'Your power bill is due Friday',
    sender: 'bills@power.example',
    screening: { verdict: 'clean', reasons: [], links: [] },
    status: 'pending',
    resultRequestId: null,
    threadId: 'intake:q1',
    plan: null,
    ...overrides,
  }
}

const thread = (items: QueuedWork[]): IntakeThread => ({
  conversationId: 'intake:q1',
  title: 'Power bill',
  source: 'email',
  items,
  requestIds: [],
  workflowProposals: [],
})

describe('intake-thread-model', () => {
  it('splits pending items from decided ones, keeping the order', () => {
    const model = threadModel(
      thread([
        work({ id: 'a', status: 'dismissed' }),
        work({ id: 'b', status: 'approved' }),
        work({ id: 'c', status: 'pending' }),
      ]),
      'en',
      NOW
    )
    expect(model.pending.map(row => row.id)).toEqual(['c'])
    expect(model.decided.map(row => [row.id, row.statusKey])).toEqual([
      ['a', 'intakeThread.status.dismissed'],
      ['b', 'intakeThread.status.approved'],
    ])
    expect(model.sourceKey).toBe('home.chiefOfStaffDigest.source.email')
  })

  it('blocks approval of a blocked pending item', () => {
    const model = threadModel(
      thread([
        work({
          screening: { verdict: 'blocked', reasons: ['Malware'], links: [] },
        }),
      ]),
      'en',
      NOW
    )
    expect(model.pending[0]).toMatchObject({
      canApprove: false,
      reasons: ['Malware'],
    })
  })

  it('confirms a suspicious item and shows its plan', () => {
    const model = threadModel(
      thread([
        work({
          screening: {
            verdict: 'suspicious',
            reasons: ['Asks for a password'],
            links: [],
          },
          plan: {
            steps: [{ kind: 'localAgent', target: 'a', description: 'Pay it' }],
            estimatedDurationSeconds: null,
            estimatedCostUsd: null,
            warnings: [],
            rejection: null,
          },
        }),
      ]),
      'en',
      NOW
    )
    expect(model.pending[0]).toMatchObject({
      needsConfirm: true,
      planSteps: ['Pay it'],
    })
  })

  it('has nothing pending or decided for an empty thread', () => {
    const model = threadModel(thread([]), 'en', NOW)
    expect(model.pending).toEqual([])
    expect(model.decided).toEqual([])
  })

  it("offers Mark done on the user's own Requests only", () => {
    const local = { direction: 'local', capability: '' } as Request
    expect(canMarkStepsDone(local)).toBe(true)
    expect(
      canMarkStepsDone({ direction: 'incoming', capability: 'x' } as Request)
    ).toBe(false)
  })

  it('finds the workflow proposal for one of its Requests', () => {
    const proposal = {
      requestId: 'r-nov',
      reason: 'sameWork' as const,
      likeRequestId: 'r-oct',
      likeTitle: 'Pay the power bill for October',
    }
    const withProposal = { ...thread([work()]), workflowProposals: [proposal] }
    expect(workflowProposalFor(withProposal, 'r-nov')).toEqual(proposal)
    expect(workflowProposalFor(withProposal, 'r-oct')).toBeNull()
  })
})
