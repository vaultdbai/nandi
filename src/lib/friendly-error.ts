/**
 * Raw backend errors in plain words. The UI shows the sentence; the raw
 * text stays behind "Details" (`ErrorMessage`, `toastError`).
 */
import { PEER_KEY_CHANGED, RELAY_UNAVAILABLE } from '@/lib/bindings'

export type ErrorKind =
  | 'relayUnavailable'
  | 'peerKeyChanged'
  | 'offline'
  | 'timeout'
  | 'notFound'
  | 'auth'
  | 'permission'
  | 'conflict'
  | 'invalid'
  | 'busy'
  | 'other'

/** Checked in order: the first pattern that matches names the kind */
const PATTERNS: [ErrorKind, RegExp][] = [
  ['timeout', /timed? ?out|deadline exceeded/i],
  [
    'offline',
    /couldn.t (reach|connect)|connection (refused|reset|closed)|unreachable|network|dns|resolve host|host not found|offline|ECONN|failed to fetch/i,
  ],
  [
    'auth',
    /\b401\b|unauthori[sz]ed|authentication|sign(ed)? ?in|log ?in again|credentials?|token (expired|invalid)|api key/i,
  ],
  ['permission', /\b403\b|forbidden|permission|access denied|not allowed/i],
  [
    'notFound',
    /\b404\b|not found|wasn.t found|no such|doesn.t exist|does not exist/i,
  ],
  ['conflict', /already (exists|subscribed|installed)|conflict|\b409\b/i],
  [
    'invalid',
    /invalid|parse|yaml|json|expected|unknown (key|field)|missing field|must (be|use|start)/i,
  ],
  ['busy', /\b429\b|rate.?limit|busy|too many/i],
]

/** The raw text of anything a command rejected with */
export function rawError(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  // `unwrapResult` throws the error `Result` itself
  if (error && typeof error === 'object' && 'error' in error) {
    return rawError((error as { error: unknown }).error)
  }
  return String(error)
}

/**
 * Whether a person-to-person action answered "no relay connected"
 * (`RelayUnavailable` in Rust): the place shows `RelayConnectNotice`
 * instead of an error.
 */
export function isRelayUnavailable(error: unknown): boolean {
  return rawError(error).startsWith(RELAY_UNAVAILABLE)
}

/**
 * Whether a send was refused because the member's key changed and the
 * user hasn't accepted it (`network::peer_keys` in Rust)
 */
export function isPeerKeyChanged(error: unknown): boolean {
  return rawError(error).startsWith(PEER_KEY_CHANGED)
}

export function errorKind(error: unknown): ErrorKind {
  if (isRelayUnavailable(error)) return 'relayUnavailable'
  if (isPeerKeyChanged(error)) return 'peerKeyChanged'
  const raw = rawError(error)
  return PATTERNS.find(([, pattern]) => pattern.test(raw))?.[0] ?? 'other'
}

/** The `errors.*` key for the plain sentence */
export function friendlyErrorKey(error: unknown): string {
  return `errors.${errorKind(error)}`
}
