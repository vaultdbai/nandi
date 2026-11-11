import { describe, expect, it } from 'vitest'
import type { TrialAccount, TrialStatus } from '@/lib/bindings'
import { trialLinkLine, trialMessage, trialUsedUp } from './free-trial'

const trial = (overrides: Partial<TrialStatus> = {}): TrialStatus => ({
  enabled: true,
  active: false,
  usedUp: false,
  paused: false,
  unreachable: false,
  ...overrides,
})

const account = (overrides: Partial<TrialAccount> = {}): TrialAccount => ({
  account: null,
  email: null,
  used: 0,
  limit: 100_000,
  paused: false,
  ...overrides,
})

describe('the free trial card', () => {
  it('uses the gateway’s count', () => {
    expect(
      trialMessage(trial({ active: true }), account({ used: 40_000 }), false)
    ).toEqual({ key: 'active', left: 60_000, limit: 100_000 })
    expect(trialMessage(trial(), account(), false).key).toBe('standby')
    expect(trialMessage(trial({ enabled: false }), account(), false).key).toBe(
      'off'
    )
  })

  it('says used up, paused, unreachable or checking', () => {
    expect(trialMessage(trial(), account({ used: 100_000 }), false).key).toBe(
      'usedUp'
    )
    expect(trialMessage(trial({ usedUp: true }), undefined, false).key).toBe(
      'usedUp'
    )
    expect(trialMessage(trial(), account({ paused: true }), false).key).toBe(
      'paused'
    )
    expect(trialMessage(trial(), undefined, true).key).toBe('unreachable')
    expect(
      trialMessage(trial({ unreachable: true }), undefined, false).key
    ).toBe('unreachable')
    expect(trialMessage(trial(), undefined, false).key).toBe('checking')
  })

  it('can’t be turned on once used up', () => {
    expect(trialUsedUp(trial(), account({ used: 100_000 }))).toBe(true)
    expect(trialUsedUp(trial({ usedUp: true }), undefined)).toBe(true)
    // The gateway's count wins over the last call's answer
    expect(trialUsedUp(trial({ usedUp: true }), account({ used: 5 }))).toBe(
      false
    )
  })

  it('names the linked email, else counts per computer', () => {
    expect(
      trialLinkLine(account({ account: 'google:1', email: 'ada@example.com' }))
    ).toEqual({ key: 'linked', email: 'ada@example.com' })
    expect(trialLinkLine(account())).toEqual({ key: 'perComputer' })
  })
})
