import { describe, expect, it } from 'vitest'
import type { GatewayCard, MemberProfile, TaskRecord } from '@/lib/bindings'
import {
  askableSteps,
  canReceiveChecks,
  canSendChecks,
  lastRequestToThem,
  verdict,
} from './member-diagnosis'

const card = (overrides: Partial<GatewayCard> = {}): GatewayCard => ({
  instanceId: 'desktop-priya',
  endpoint: '',
  displayName: "Priya's Nandi",
  description: '',
  capabilities: ['summarize-paper'],
  acceptsDelegation: true,
  maxConcurrent: 3,
  currentLoad: 0,
  owner: { githubHandle: 'priya', displayName: 'Priya' },
  federationEnabled: false,
  gatewayVersion: 1,
  directDelegation: true,
  ...overrides,
})

const task = (overrides: Partial<TaskRecord>): TaskRecord => ({
  id: 't',
  requesterInstance: 'desktop-me',
  requesterAgent: 'desktop-me/writer',
  providerInstance: 'desktop-priya',
  providerAgent: 'desktop-priya/papers',
  capability: 'summarize-paper',
  input: null,
  state: 'completed',
  etaSeconds: null,
  deadline: null,
  priority: 0,
  createdAt: 10,
  updatedAt: 10,
  acknowledgedAt: null,
  completedAt: null,
  error: null,
  result: null,
  parentTaskId: null,
  sourceKind: 'direct',
  attemptNumber: 1,
  previousAttemptId: null,
  ...overrides,
})

const profile = (overrides: Partial<MemberProfile> = {}): MemberProfile =>
  ({
    instanceId: 'desktop-priya',
    handle: 'priya',
    displayName: 'Priya',
    avatarUrl: null,
    online: true,
    lastSeen: null,
    connected: true,
    gateway: card(),
    capabilities: [],
    agents: [],
    delegationStats: { sent: 0, received: 0, completed: 0, failed: 0 },
    recentDelegations: [],
    trust: {
      instanceId: 'desktop-priya',
      addedAsTrustedPeer: false,
      delegationsPaused: false,
      blockedAgents: [],
      lastUpdated: null,
    },
    ...overrides,
  }) as MemberProfile

const status = (p: MemberProfile) =>
  Object.fromEntries(canSendChecks(p, 'desktop-me').map(c => [c.key, c.status]))

describe('canSendChecks', () => {
  it('passes for an online, listed member who took my last request', () => {
    const p = profile({
      recentDelegations: [task({ state: 'completed' })],
    })
    expect(status(p)).toEqual({
      online: 'pass',
      listed: 'pass',
      general: 'pass',
      byName: 'pass',
      trustsMe: 'pass',
      notPaused: 'pass',
    })
    expect(verdict(canSendChecks(p, 'desktop-me'))).toBe(true)
  })

  it('reads "trusts me" from my last request: unknown until I ask', () => {
    expect(status(profile()).trustsMe).toBe('unknown')
    const refused = profile({
      recentDelegations: [
        task({ id: 'old', createdAt: 1, state: 'completed' }),
        task({
          id: 'new',
          createdAt: 2,
          state: 'rejected',
          error:
            'The owner of Papers needs to trust Alex before it takes their requests.',
        }),
      ],
    })
    expect(status(refused).trustsMe).toBe('fail')
    // Requests they sent me say nothing about whether they trust me
    const theirs = profile({
      recentDelegations: [
        task({
          requesterInstance: 'desktop-priya',
          providerInstance: 'desktop-me',
          state: 'rejected',
          error: 'trust',
        }),
      ],
    })
    expect(status(theirs).trustsMe).toBe('unknown')
  })

  it('fails offline, unlisted, not taking requests, and paused by me', () => {
    expect(status(profile({ online: false })).online).toBe('fail')
    expect(status(profile({ gateway: null }))).toMatchObject({
      listed: 'fail',
      general: 'fail',
      byName: 'unknown',
    })
    expect(
      status(profile({ gateway: card({ acceptsDelegation: false }) })).general
    ).toBe('fail')
    expect(
      status(profile({ gateway: card({ directDelegation: false }) })).byName
    ).toBe('fail')
    expect(
      status(profile({ gateway: card({ directDelegation: null }) })).byName
    ).toBe('unknown')
    const paused = profile({
      trust: { ...profile().trust, delegationsPaused: true },
    })
    expect(status(paused).notPaused).toBe('fail')
    expect(verdict(canSendChecks(paused, 'desktop-me'))).toBe(false)
  })

  it('asks them only for what they can fix', () => {
    const p = profile({
      online: false,
      gateway: card({ acceptsDelegation: false }),
      recentDelegations: [
        task({ state: 'rejected', error: 'needs to trust you' }),
      ],
    })
    expect(askableSteps(canSendChecks(p, 'desktop-me'))).toEqual([
      'general',
      'trustsMe',
    ])
  })
})

describe('canReceiveChecks', () => {
  it('needs my Nandi reachable and them not paused; trust is optional', () => {
    const checks = canReceiveChecks(profile(), true)
    expect(checks.map(c => [c.key, c.status])).toEqual([
      ['myReachable', 'pass'],
      ['iTrust', 'unknown'],
      ['notPausedByMe', 'pass'],
    ])
    expect(verdict(checks)).toBe(true)
    expect(verdict(canReceiveChecks(profile(), false))).toBe(false)
    const trusted = profile({
      trust: { ...profile().trust, addedAsTrustedPeer: true },
    })
    expect(canReceiveChecks(trusted, true)[1]?.status).toBe('pass')
  })

  it('counts everyone on a secured relay as trusted when that is on', () => {
    expect(canReceiveChecks(profile(), true, true)[1]?.status).toBe('pass')
    expect(canReceiveChecks(profile(), true, false)[1]?.status).toBe('unknown')
  })
})

describe('lastRequestToThem', () => {
  it('is null without their instance', () => {
    expect(lastRequestToThem([task({})], 'desktop-me', null)).toBeNull()
  })
})
