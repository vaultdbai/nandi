/**
 * What the user reads after "Join": one calm line per outcome of
 * `joinRelay`, plus the relay's own words where it gave any (a full relay's
 * free-limit message is shown verbatim). Never the join code or a token.
 */
import type { JoinOutcome } from '@/lib/bindings'

export interface JoinMessage {
  /** i18n key */
  key: string
  params: Record<string, string | number>
  /** The relay's words or the technical detail, shown under the line */
  detail: string | null
  ok: boolean
}

/** `relay.acme.com` from `wss://relay.acme.com/relay` */
function host(url: string): string {
  return /^wss?:\/\/([^/\s?#]+)/i.exec(url.trim())?.[1] ?? url
}

export function joinMessage(outcome: JoinOutcome, url: string): JoinMessage {
  const base = { params: { host: host(url) }, detail: null, ok: false }
  switch (outcome.kind) {
    case 'joined':
      return {
        ...base,
        key: 'relay.join.joined',
        params: { host: host(outcome.relay) },
        ok: true,
      }
    case 'invalidUrl':
      return { ...base, key: 'relay.join.invalidUrl' }
    case 'unreachable':
      return { ...base, key: 'relay.join.unreachable', detail: outcome.detail }
    case 'versionMismatch':
      return {
        ...base,
        key: 'relay.join.versionMismatch',
        params: { host: host(url), relay: outcome.relay, app: outcome.app },
      }
    case 'codeNeeded':
      return { ...base, key: 'relay.join.codeNeeded' }
    case 'wrongCode':
      return { ...base, key: 'relay.join.wrongCode' }
    case 'relayFull':
      return { ...base, key: 'relay.join.relayFull', detail: outcome.message }
    case 'alreadyEnrolled':
      return {
        ...base,
        key: 'relay.join.alreadyEnrolled',
        detail: outcome.message,
      }
    case 'removed':
      return { ...base, key: 'relay.join.removed', detail: outcome.message }
    case 'noEnrollment':
      return { ...base, key: 'relay.join.noEnrollment' }
    case 'refused':
      return { ...base, key: 'relay.join.refused', detail: outcome.message }
  }
}
