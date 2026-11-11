import { describe, expect, it } from 'vitest'
import { afterJoin, onPublicRelay, refusalKey } from './public-relay-model'
import { relayStatusFixture } from './network-fixtures'

describe('joining the public relay', () => {
  it('ends joined with the email', () => {
    expect(
      afterJoin({
        kind: 'joined',
        relay: 'wss://relay.vaultdb.ai/relay',
        account: 'google:1099',
        email: 'ada@example.com',
        name: 'Ada',
      })
    ).toEqual({
      phase: 'joined',
      email: 'ada@example.com',
      relay: 'wss://relay.vaultdb.ai/relay',
    })
  })

  it('asks for an email when there is none to join with', () => {
    expect(
      afterJoin({ kind: 'needsEmail', message: 'Connect ada@example.com' })
    ).toEqual({ phase: 'needsEmail', message: 'Connect ada@example.com' })
  })

  it('names refusals and an unreachable relay calmly', () => {
    expect(
      afterJoin({
        kind: 'refused',
        refusal: 'machineLimit',
        message: 'One computer joins at most 5 Nandis',
      })
    ).toEqual({
      phase: 'failed',
      failure: {
        key: 'publicRelay.refusal.machineLimit',
        detail: 'One computer joins at most 5 Nandis',
      },
    })
    expect(afterJoin({ kind: 'unreachable', detail: 'refused' })).toEqual({
      phase: 'failed',
      failure: { key: 'publicRelay.unreachable', detail: 'refused' },
    })
    expect(refusalKey('deviceLimit')).toBe('publicRelay.refusal.deviceLimit')
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
