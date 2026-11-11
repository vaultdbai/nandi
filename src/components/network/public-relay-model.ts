/**
 * "Join the public relay": what joining leads to and what each outcome
 * says, pure so onboarding and Network › Connection share them (and tests
 * drive them). There is no sign-in: the core joins with this Nandi's
 * connected email (`services::relay::public`); this decides what the user
 * sees next.
 */
import type {
  PublicRelayJoin,
  PublicRelayRefusal,
  RelayStatus,
} from '@/lib/bindings'
import { withoutToken } from '@/components/onboarding/identity-model'

/** A failure in one calm line: an i18n key and the relay's words */
export interface JoinFailure {
  key: string
  detail: string | null
}

export type JoinState =
  | { phase: 'idle' }
  | { phase: 'joining' }
  | { phase: 'joined'; email: string; relay: string }
  /** No usable email: connect one in Settings › Email (`message` says how) */
  | { phase: 'needsEmail'; message: string }
  | { phase: 'failed'; failure: JoinFailure }

export const IDLE: JoinState = { phase: 'idle' }

/** The i18n key for a refusal (`publicRelay.refusal.*`) */
export function refusalKey(refusal: PublicRelayRefusal): string {
  return `publicRelay.refusal.${refusal}`
}

/** What joining led to */
export function afterJoin(join: PublicRelayJoin): JoinState {
  switch (join.kind) {
    case 'joined':
      return { phase: 'joined', email: join.email, relay: join.relay }
    case 'needsEmail':
      return { phase: 'needsEmail', message: join.message }
    case 'refused':
      return {
        phase: 'failed',
        failure: { key: refusalKey(join.refusal), detail: join.message },
      }
    case 'unreachable':
      return {
        phase: 'failed',
        failure: { key: 'publicRelay.unreachable', detail: join.detail },
      }
  }
}

/** Whether the relay in use is the public relay the app offers */
export function onPublicRelay(status: RelayStatus | null | undefined): boolean {
  const url = status?.relayUrl
  const offered = status?.publicRelayUrl
  if (!url || !offered) return false
  return withoutToken(url) === withoutToken(offered)
}
