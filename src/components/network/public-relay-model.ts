/**
 * "Join the public relay": the sign-in's states and what each says, pure
 * so onboarding and Network › Connection share them (and tests drive them
 * without a timer). The core does the talking (`services::relay::public`);
 * this decides what the user sees next.
 */
import type {
  PublicRelayRefusal,
  PublicSignInPoll,
  PublicSignInStart,
  RelaySignInProvider,
  RelayStatus,
} from '@/lib/bindings'
import { withoutToken } from '@/components/onboarding/identity-model'

/** The providers in the order the buttons show them */
export const RELAY_SIGN_IN_PROVIDERS: readonly RelaySignInProvider[] = [
  'github',
  'google',
  'microsoft',
]

/** A failure in one calm line: an i18n key and the relay's words */
export interface SignInFailure {
  key: string
  detail: string | null
}

export type SignInState =
  | { phase: 'choose' }
  | { phase: 'starting'; provider: RelaySignInProvider }
  | {
      phase: 'waiting'
      provider: RelaySignInProvider
      signinId: string
      userCode: string
      /** The page to open: the one with the code filled in when offered */
      page: string
      /** Seconds until the next poll */
      interval: number
      /** When the code expires (ms since the epoch) */
      expiresAt: number
      /** The last poll didn't reach the relay; still waiting */
      retrying: boolean
    }
  | {
      phase: 'linked'
      provider: RelaySignInProvider
      display: string
      relay: string
    }
  | { phase: 'failed'; failure: SignInFailure }

export const CHOOSE: SignInState = { phase: 'choose' }

/** The i18n key for a refusal (`publicRelay.refusal.*`) */
export function refusalKey(refusal: PublicRelayRefusal): string {
  return `publicRelay.refusal.${refusal}`
}

/** The i18n key naming a provider */
export function providerKey(provider: RelaySignInProvider): string {
  return `publicRelay.provider.${provider}`
}

/** Providers the relay offers, in button order (unknown ones dropped) */
export function offeredProviders(
  offered: readonly RelaySignInProvider[]
): RelaySignInProvider[] {
  return RELAY_SIGN_IN_PROVIDERS.filter(provider => offered.includes(provider))
}

/** What starting a sign-in leads to */
export function afterStart(
  provider: RelaySignInProvider,
  start: PublicSignInStart,
  now: number
): SignInState {
  switch (start.kind) {
    case 'started':
      return {
        phase: 'waiting',
        provider,
        signinId: start.signinId,
        userCode: start.userCode,
        page: start.verificationUriComplete ?? start.verificationUri,
        interval: Math.max(1, start.interval),
        expiresAt: now + start.expiresIn * 1000,
        retrying: false,
      }
    case 'refused':
      return {
        phase: 'failed',
        failure: { key: refusalKey(start.refusal), detail: start.message },
      }
    case 'unreachable':
      return {
        phase: 'failed',
        failure: { key: 'publicRelay.unreachable', detail: start.detail },
      }
  }
}

const EXPIRED: SignInFailure = {
  key: refusalKey('signinExpired'),
  detail: null,
}

/**
 * What a poll leads to. A relay that didn't answer this once keeps the
 * sign-in waiting (it's only polling) until the code expires.
 */
export function afterPoll(
  state: SignInState,
  poll: PublicSignInPoll,
  now: number
): SignInState {
  if (state.phase !== 'waiting') return state
  switch (poll.kind) {
    case 'pending':
    case 'slowDown':
      if (now >= state.expiresAt) return { phase: 'failed', failure: EXPIRED }
      return { ...state, interval: Math.max(1, poll.interval), retrying: false }
    case 'unreachable':
      if (now >= state.expiresAt) return { phase: 'failed', failure: EXPIRED }
      return { ...state, retrying: true }
    case 'linked':
      return {
        phase: 'linked',
        provider: poll.provider,
        display: poll.display,
        relay: poll.relay,
      }
    case 'refused':
      return {
        phase: 'failed',
        failure: { key: refusalKey(poll.refusal), detail: poll.message },
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
