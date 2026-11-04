import { describe, expect, it } from 'vitest'
import type { Blocker, BlockerKind, Request } from '@/lib/bindings'
import { changesRequests } from '@/hooks/useRequests'
import {
  askOf,
  bannerActions,
  bannerSentenceKey,
  bannerTone,
  canStop,
  isGatewayHold,
  needsConfirmation,
  stepDuration,
  stepNumbers,
} from './request-model'
import { blocker, request } from './request-fixtures'

describe('bannerActions', () => {
  const actions = (
    kind: BlockerKind,
    direction: Request['direction'],
    extra: Partial<Blocker> = {}
  ) => {
    const b = blocker(kind, extra)
    return bannerActions(request({ direction, stuck: b }), b)
  }

  it('gives every blocker kind its fix (request-detail §5.2)', () => {
    expect(
      actions('needsMyApproval', 'incoming', { approvalId: 'ap1' })
    ).toEqual({
      primary: 'approve',
      secondary: ['reject'],
    })
    expect(actions('needsMyApproval', 'incoming').primary).toBe('details')
    expect(
      actions('needsTheirApproval', 'outgoing', { terminal: true })
    ).toEqual({
      primary: 'remind',
      secondary: [],
    })
    expect(actions('needsMyAnswer', 'outgoing').primary).toBe('answer')
    expect(actions('needsTheirAnswer', 'incoming')).toEqual({
      primary: null,
      secondary: ['cancel'],
    })
    expect(actions('notTrusted', 'incoming')).toEqual({
      primary: 'trust',
      secondary: ['block'],
    })
    expect(actions('notTrusted', 'outgoing').primary).toBe('askTrust')
    expect(
      actions('notAccepting', 'incoming', { agentId: 'papers' }).primary
    ).toBe('turnOn')
    expect(actions('notAccepting', 'outgoing').primary).toBe('askTurnOn')
    expect(actions('pausedOrBlocked', 'incoming').primary).toBe('resume')
    expect(actions('queued', 'incoming')).toEqual({
      primary: null,
      secondary: ['cancel'],
    })
    expect(actions('offline', 'outgoing')).toEqual({
      primary: 'sendElsewhere',
      secondary: ['cancel'],
    })
    expect(actions('noModel', 'incoming').primary).toBe('addModel')
    expect(actions('runtimeAuth', 'incoming').primary).toBe('addKey')
    expect(actions('timedOut', 'outgoing').primary).toBe('retry')
    expect(actions('failed', 'outgoing')).toEqual({
      primary: 'retry',
      secondary: ['details'],
    })
    // The requester decides about retrying; the provider can only look
    expect(actions('failed', 'incoming').primary).toBe('details')
  })
})

describe('confirmations (principle 7)', () => {
  it('asks before reject, trust, block and cancel only', () => {
    for (const action of ['reject', 'trust', 'block', 'cancel'] as const) {
      expect(needsConfirmation(action)).toBe(true)
    }
    for (const action of [
      'approve',
      'answer',
      'turnOn',
      'resume',
      'addModel',
      'retry',
    ] as const) {
      expect(needsConfirmation(action)).toBe(false)
    }
  })
})

describe('banner tone', () => {
  it('is warning on me, neutral on others, destructive once ended', () => {
    expect(bannerTone(blocker('needsMyApproval'))).toBe('warning')
    expect(bannerTone(blocker('needsTheirAnswer', { who: 'them' }))).toBe(
      'neutral'
    )
    expect(bannerTone(blocker('failed', { terminal: true }))).toBe(
      'destructive'
    )
  })
})

describe('steps', () => {
  it('numbers plan steps only', () => {
    const numbers = stepNumbers(request().steps)
    expect(numbers.get('received')).toBeUndefined()
    expect(numbers.get('step-1')).toBe(1)
  })

  it('times steps to now while they run', () => {
    expect(stepDuration({ startedAt: 1_000, endedAt: null }, 4_000)).toBe(3_000)
    expect(stepDuration({ startedAt: 1_000, endedAt: 2_000 }, 9_000)).toBe(
      1_000
    )
    expect(stepDuration({ startedAt: null, endedAt: null }, 9_000)).toBeNull()
  })

  it('offers Stop while it runs or waits', () => {
    expect(canStop(request({ status: 'running' }))).toBe(true)
    expect(canStop(request({ status: 'waiting' }))).toBe(true)
    expect(canStop(request({ status: 'done' }))).toBe(false)
  })
})

describe('live updates', () => {
  it('refreshes on the events that change a Request', () => {
    for (const event of [
      'task://state_changed',
      'task://progress',
      'work://item_updated',
      'agent://approval_resolved',
    ]) {
      expect(changesRequests(event)).toBe(true)
    }
    expect(changesRequests('repo://agent.updated')).toBe(false)
  })
})

describe('held and declined requests (task 5)', () => {
  it('lets me approve or reject a request held at the approval step', () => {
    const b = blocker('needsMyApproval', { stepId: 'approval' })
    const r = request({ direction: 'incoming', stuck: b })
    expect(isGatewayHold(r, b)).toBe(true)
    expect(bannerActions(r, b)).toEqual({
      primary: 'approve',
      secondary: ['reject'],
    })
    expect(bannerSentenceKey(r, b)).toBe('request.blocker.hold.incoming')
  })

  it('is not a hold on the outgoing side or another step', () => {
    const b = blocker('needsMyApproval', { stepId: 'agent' })
    expect(isGatewayHold(request({ direction: 'incoming', stuck: b }), b)).toBe(
      false
    )
    const held = blocker('needsMyApproval', { stepId: 'approval' })
    expect(
      isGatewayHold(request({ direction: 'outgoing', stuck: held }), held)
    ).toBe(false)
  })

  it('offers to send a declined request elsewhere', () => {
    const b = blocker('declined', { terminal: true })
    expect(
      bannerActions(request({ direction: 'outgoing', stuck: b }), b).primary
    ).toBe('sendElsewhere')
    expect(
      bannerSentenceKey(request({ direction: 'outgoing', stuck: b }), b)
    ).toBe('request.blocker.declined.outgoing')
  })
})

describe('askOf', () => {
  it('keeps the words asked as written, and every other input', () => {
    expect(
      askOf({
        text: 'Review the SOC 1 draft\nFocus on section 4',
        due: '2026-11-03',
        links: [{ url: 'https://x.test/doc', name: 'Draft' }],
        priority: 2,
      })
    ).toEqual({
      text: 'Review the SOC 1 draft\nFocus on section 4',
      fields: [
        { key: 'due', value: '2026-11-03' },
        { key: 'links', value: 'Draft (https://x.test/doc)' },
        { key: 'priority', value: '2' },
      ],
    })
  })

  it('reads a plain string, other text keys and structured-only inputs', () => {
    expect(askOf('Summarize Q3')).toEqual({ text: 'Summarize Q3', fields: [] })
    expect(askOf({ query: 'risk' }).text).toBe('risk')
    expect(askOf({ ticker: 'ACME' })).toEqual({
      text: null,
      fields: [{ key: 'ticker', value: 'ACME' }],
    })
    expect(askOf(null)).toEqual({ text: null, fields: [] })
    expect(askOf(undefined)).toEqual({ text: null, fields: [] })
  })
})
