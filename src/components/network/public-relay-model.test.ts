import { describe, expect, it } from 'vitest'
import {
  afterPoll,
  afterStart,
  CHOOSE,
  offeredProviders,
  onPublicRelay,
  refusalKey,
  type SignInState,
} from './public-relay-model'
import { relayStatusFixture } from './network-fixtures'

const NOW = 1_000_000

const started = afterStart(
  'github',
  {
    kind: 'started',
    signinId: 's-1',
    provider: 'github',
    userCode: 'WDJB-MJHT',
    verificationUri: 'https://github.com/login/device',
    verificationUriComplete: null,
    expiresIn: 900,
    interval: 5,
  },
  NOW
)

describe('public relay sign-in', () => {
  it('starts waiting with the code, the page and the interval', () => {
    expect(started).toEqual({
      phase: 'waiting',
      provider: 'github',
      signinId: 's-1',
      userCode: 'WDJB-MJHT',
      page: 'https://github.com/login/device',
      interval: 5,
      expiresAt: NOW + 900_000,
      retrying: false,
    })
    // The page with the code filled in, when the provider has one
    const complete = afterStart(
      'microsoft',
      {
        kind: 'started',
        signinId: 's-2',
        provider: 'microsoft',
        userCode: 'ABC',
        verificationUri: 'https://microsoft.com/devicelogin',
        verificationUriComplete: 'https://microsoft.com/devicelogin?code=ABC',
        expiresIn: 600,
        interval: 0,
      },
      NOW
    )
    expect(complete).toMatchObject({
      page: 'https://microsoft.com/devicelogin?code=ABC',
      interval: 1,
    })
  })

  it('says why a start failed', () => {
    expect(
      afterStart(
        'google',
        { kind: 'refused', refusal: 'signinLimit', message: 'Too many' },
        NOW
      )
    ).toEqual({
      phase: 'failed',
      failure: { key: 'publicRelay.refusal.signinLimit', detail: 'Too many' },
    })
    expect(
      afterStart('github', { kind: 'unreachable', detail: 'refused' }, NOW)
    ).toEqual({
      phase: 'failed',
      failure: { key: 'publicRelay.unreachable', detail: 'refused' },
    })
  })

  it('keeps waiting at the interval the relay asks for', () => {
    expect(afterPoll(started, { kind: 'pending', interval: 5 }, NOW)).toEqual(
      started
    )
    expect(
      afterPoll(started, { kind: 'slowDown', interval: 10 }, NOW)
    ).toMatchObject({ phase: 'waiting', interval: 10 })
  })

  it('rides out a relay that missed one poll, until the code expires', () => {
    const retrying = afterPoll(
      started,
      { kind: 'unreachable', detail: 'reset' },
      NOW
    )
    expect(retrying).toMatchObject({ phase: 'waiting', retrying: true })
    expect(
      afterPoll(retrying, { kind: 'pending', interval: 5 }, NOW)
    ).toMatchObject({ retrying: false })
    const late = NOW + 900_000
    for (const poll of [
      { kind: 'pending', interval: 5 },
      { kind: 'unreachable', detail: 'reset' },
    ] as const)
      expect(afterPoll(started, poll, late)).toEqual({
        phase: 'failed',
        failure: { key: 'publicRelay.refusal.signinExpired', detail: null },
      })
  })

  it('ends linked, or with the refusal', () => {
    expect(
      afterPoll(
        started,
        {
          kind: 'linked',
          relay: 'wss://relay.vaultdb.ai/relay',
          account: 'github:1',
          provider: 'github',
          display: 'octocat',
        },
        NOW
      )
    ).toEqual({
      phase: 'linked',
      provider: 'github',
      display: 'octocat',
      relay: 'wss://relay.vaultdb.ai/relay',
    })
    expect(
      afterPoll(
        started,
        { kind: 'refused', refusal: 'signinDenied', message: 'Cancelled' },
        NOW
      )
    ).toEqual({
      phase: 'failed',
      failure: { key: 'publicRelay.refusal.signinDenied', detail: 'Cancelled' },
    })
    // Only a waiting sign-in moves
    const choosing: SignInState = CHOOSE
    expect(afterPoll(choosing, { kind: 'pending', interval: 5 }, NOW)).toBe(
      choosing
    )
  })

  it('names refusals and orders providers', () => {
    expect(refusalKey('deviceLimit')).toBe('publicRelay.refusal.deviceLimit')
    expect(offeredProviders(['microsoft', 'github'])).toEqual([
      'github',
      'microsoft',
    ])
  })

  it('knows when the relay in use is the public one', () => {
    const PUBLIC = 'wss://relay.vaultdb.ai/relay'
    expect(
      onPublicRelay(
        relayStatusFixture({
          relayUrl: `${PUBLIC}?token=t`,
          publicRelayUrl: PUBLIC,
        })
      )
    ).toBe(true)
    expect(onPublicRelay(relayStatusFixture({ publicRelayUrl: PUBLIC }))).toBe(
      false
    )
    expect(
      onPublicRelay(
        relayStatusFixture({ relayUrl: null, publicRelayUrl: PUBLIC })
      )
    ).toBe(false)
    expect(onPublicRelay(undefined)).toBe(false)
  })
})
