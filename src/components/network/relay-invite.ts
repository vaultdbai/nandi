/**
 * The invite a member copies for a teammate, and reading one back.
 *
 * The invite line is the relay address, with the join code (when there is
 * one) in the fragment: `wss://relay.acme.com/relay#join=acme-2026`. The
 * fragment never reaches a server, and the line reads the same in every
 * language, so "Paste invite" works whatever language the message was
 * copied in. It never carries an instance token.
 */
import type { RelayInvite } from '@/lib/bindings'

const JOIN_PARAM = 'join'

/** The line a teammate pastes: the address, plus `#join=<code>` if any */
export function inviteLine(invite: RelayInvite): string {
  const url = withoutTokenOrFragment(invite.url)
  const code = invite.joinCode?.trim()
  return code ? `${url}#${JOIN_PARAM}=${encodeURIComponent(code)}` : url
}

export interface PastedInvite {
  url: string
  /** `null`: the invite carries no code (an open relay, or none given) */
  joinCode: string | null
}

/**
 * The relay and join code in pasted text: a whole invite message, or just
 * its line. `null` when there is no `ws://` or `wss://` address in it. A
 * `?token=` someone pasted by mistake is dropped.
 */
export function parseInvite(text: string): PastedInvite | null {
  const match = /wss?:\/\/[^\s"'<>]+/i.exec(text)
  if (!match) return null
  const [address = '', fragment = ''] = match[0].split('#', 2)
  const url = withoutTokenOrFragment(address).replace(/[.,;:)]+$/, '')
  const code = new URLSearchParams(fragment).get(JOIN_PARAM)?.trim()
  return { url, joinCode: code ? code : null }
}

/**
 * Whether typed or pasted text is an invite rather than an address being
 * typed: a message (it has spaces or lines) or an address with a join code
 */
export function isInviteText(text: string): boolean {
  const invite = parseInvite(text)
  if (!invite) return false
  return invite.joinCode !== null || /\s/.test(text.trim())
}

function withoutTokenOrFragment(url: string): string {
  const [base = ''] = url.trim().split('#', 1)
  const [path = '', query] = base.split('?', 2)
  if (!query) return path
  const rest = query.split('&').filter(pair => !pair.startsWith('token='))
  return rest.length ? `${path}?${rest.join('&')}` : path
}
