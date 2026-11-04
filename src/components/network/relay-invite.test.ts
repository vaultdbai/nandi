import { describe, expect, it } from 'vitest'
import { inviteLine, isInviteText, parseInvite } from './relay-invite'

describe('relay invites', () => {
  it('puts the join code in the fragment, encoded', () => {
    expect(
      inviteLine({ url: 'wss://relay.acme.com/relay', joinCode: 'acme 2026' })
    ).toBe('wss://relay.acme.com/relay#join=acme%202026')
    expect(
      inviteLine({ url: 'wss://relay.acme.com/relay', joinCode: null })
    ).toBe('wss://relay.acme.com/relay')
  })

  it('never carries an instance token', () => {
    expect(
      inviteLine({
        url: 'wss://relay.acme.com/relay?token=s3cret',
        joinCode: 'c',
      })
    ).toBe('wss://relay.acme.com/relay#join=c')
    expect(parseInvite('ws://h:8787/relay?token=s3cret&x=1')).toEqual({
      url: 'ws://h:8787/relay?x=1',
      joinCode: null,
    })
  })

  it('reads an invite back from a whole message, in any language', () => {
    const line = inviteLine({
      url: 'wss://relay.acme.com/relay',
      joinCode: 'acme 2026',
    })
    expect(parseInvite(`Rejoins mon équipe sur Nandi :\n\n${line}`)).toEqual({
      url: 'wss://relay.acme.com/relay',
      joinCode: 'acme 2026',
    })
    expect(parseInvite('Use ws://10.0.0.5:8787/relay.')).toEqual({
      url: 'ws://10.0.0.5:8787/relay',
      joinCode: null,
    })
    expect(parseInvite('no address here')).toBeNull()
  })

  it('tells an invite from an address being typed', () => {
    expect(isInviteText('wss://relay.acme.com/relay#join=c')).toBe(true)
    expect(isInviteText('Join me:\nws://h/relay')).toBe(true)
    expect(isInviteText('ws://relay.')).toBe(false)
    expect(isInviteText('ws://relay.acme.com/relay')).toBe(false)
    expect(isInviteText('hello')).toBe(false)
  })
})
