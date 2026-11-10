import { describe, expect, it } from 'vitest'
import {
  dailyLimitResetsAt,
  errorKind,
  friendlyErrorKey,
  friendlyErrorParams,
  isPeerKeyChanged,
  isRelayUnavailable,
  rawError,
} from './friendly-error'

describe('friendly errors', () => {
  it.each([
    ['Connection refused (os error 10061)', 'offline'],
    ['Request timed out after 30s', 'timeout'],
    ['github.com/x/y wasn’t found: 404', 'notFound'],
    ['HTTP 401 Unauthorized', 'auth'],
    ['403 Forbidden: resource not accessible', 'permission'],
    ['Already subscribed to https://gitlab.com/team/caps.git', 'conflict'],
    ["steps[0]: unknown key 'agnet'", 'invalid'],
    ['429 Too Many Requests', 'busy'],
    ['Something odd happened', 'other'],
    [
      "relay_unavailable: this needs your company's relay, and Nandi isn't connected to one",
      'relayUnavailable',
    ],
    [
      "peer_key_changed: bob-1's key changed. Check the new key on their profile and accept it to send to them again",
      'peerKeyChanged',
    ],
    [
      "peer_key_missing: bob-1 hasn't published keys to the relay",
      'peerKeyMissing',
    ],
    [
      'seal_refused: a message from bob-1 about t1 was refused because it was sealed for someone else',
      'sealRefused',
    ],
    [
      'envelope_too_large: this is 2000000 bytes, over the 1048576-byte limit',
      'envelopeTooLarge',
    ],
  ])('%s → %s', (raw, kind) => {
    expect(errorKind(raw)).toBe(kind)
  })

  it('reads the raw text from whatever a command rejected with', () => {
    expect(rawError('plain')).toBe('plain')
    expect(rawError(new Error('boom'))).toBe('boom')
    expect(rawError({ status: 'error', error: 'from result' })).toBe(
      'from result'
    )
    expect(friendlyErrorKey(new Error('ECONNRESET'))).toBe('errors.offline')
  })

  it('recognises the typed "no relay connected" error however it arrives', () => {
    const text = 'relay_unavailable: this needs your company’s relay'
    expect(isRelayUnavailable(text)).toBe(true)
    expect(isRelayUnavailable(new Error(text))).toBe(true)
    expect(isRelayUnavailable({ status: 'error', error: text })).toBe(true)
    expect(isRelayUnavailable('Not connected to the network')).toBe(false)
    expect(friendlyErrorKey(text)).toBe('errors.relayUnavailable')
  })

  it('recognises a send refused because their key changed', () => {
    const text = "peer_key_changed: bob-1's key changed"
    expect(isPeerKeyChanged(new Error(text))).toBe(true)
    expect(isPeerKeyChanged('relay_unavailable: no relay')).toBe(false)
    expect(friendlyErrorKey(text)).toBe('errors.peerKeyChanged')
  })

  it('recognises a public relay daily limit and says when it resets', () => {
    const resetsAt = Date.UTC(2026, 10, 11)
    const words =
      "You've sent the 500 messages a day this relay allows (counted across your Nandis). It resets at 2026-11-11T00:00:00Z"
    // A refused request, a reassignment and a group change all carry it
    for (const raw of [
      `The relay refused it: daily_limit@${resetsAt}: ${words}`,
      `daily_limit@${resetsAt}: ${words}`,
    ]) {
      expect(errorKind(raw)).toBe('dailyLimit')
      expect(dailyLimitResetsAt(new Error(raw))).toBe(resetsAt)
    }
    expect(friendlyErrorKey(`daily_limit@${resetsAt}: x`)).toBe(
      'errors.dailyLimit'
    )
    const time = friendlyErrorParams(`daily_limit@${resetsAt}: x`, 'en').time
    expect(time).toMatch(/\d/)
    // The words alone (no code) aren't mistaken for it
    expect(dailyLimitResetsAt(words)).toBeNull()
    expect(friendlyErrorParams('timed out', 'en')).toEqual({})
  })
})
