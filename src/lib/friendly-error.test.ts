import { describe, expect, it } from 'vitest'
import {
  errorKind,
  friendlyErrorKey,
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
})
