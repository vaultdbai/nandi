import { describe, expect, it } from 'vitest'
import {
  askAvailability,
  GATEWAY_CAPABILITY,
  gatewayRequest,
  gatewayTarget,
} from './ask-model'

describe('askAvailability', () => {
  it('needs a connected, online gateway that accepts requests', () => {
    expect(askAvailability({ online: true, gateway: null })).toBe(
      'notConnected'
    )
    expect(
      askAvailability({ online: false, gateway: { acceptsDelegation: true } })
    ).toBe('offline')
    expect(
      askAvailability({ online: true, gateway: { acceptsDelegation: false } })
    ).toBe('notAccepting')
    expect(
      askAvailability({ online: true, gateway: { acceptsDelegation: true } })
    ).toBe('available')
  })
})

describe('gatewayRequest', () => {
  it("targets the member's gateway with the trimmed text", () => {
    expect(gatewayTarget('i-alice')).toBe('i-alice/gateway')
    expect(gatewayRequest('i-alice', '  find papers on X \n')).toEqual({
      capability: GATEWAY_CAPABILITY,
      input: { text: 'find papers on X' },
      target: 'i-alice/gateway',
    })
    expect(GATEWAY_CAPABILITY).toBe('gateway')
  })
})
