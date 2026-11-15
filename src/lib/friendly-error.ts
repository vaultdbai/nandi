/**
 * Raw backend errors in plain words. The UI shows the sentence; the raw
 * text stays behind "Details" (`ErrorMessage`, `toastError`).
 */
import {
  DAILY_LIMIT,
  ENVELOPE_TOO_LARGE,
  GROUP_CAPABILITY_NOT_ALLOWED,
  GROUP_DAILY_LIMIT,
  NEEDS_FIRST_TIME_APPROVAL,
  NO_SHARED_GROUP,
  PEER_KEY_CHANGED,
  PEER_KEY_MISSING,
  RELAY_NEEDS_UPDATE,
  RELAY_UNAVAILABLE,
  SEAL_REFUSED,
} from '@/lib/bindings'
import { formatResetTime } from '@/lib/format-time'

export type ErrorKind =
  | 'relayUnavailable'
  | 'relayNeedsUpdate'
  | 'peerKeyChanged'
  | 'peerKeyMissing'
  | 'sealRefused'
  | 'envelopeTooLarge'
  | 'noSharedGroup'
  | 'dailyLimit'
  | 'groupDailyLimit'
  | 'needsFirstTimeApproval'
  | 'groupCapabilityNotAllowed'
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
 * Whether a person-to-person action needs a newer relay than the one
 * connected (`RelayNeedsUpdate` in Rust): the place says "Your relay needs
 * an update for this" (`RelayConnectNotice reason="update"`) instead of an
 * error.
 */
export function isRelayNeedsUpdate(error: unknown): boolean {
  return rawError(error).startsWith(RELAY_NEEDS_UPDATE)
}

/**
 * Whether a send was refused because the member's key changed and the
 * user hasn't accepted it (`network::peer_keys` in Rust)
 */
export function isPeerKeyChanged(error: unknown): boolean {
  return rawError(error).startsWith(PEER_KEY_CHANGED)
}

/** The typed sealing refusals (`services::relay::sealing` in Rust) */
const SEALING: [ErrorKind, string][] = [
  ['peerKeyMissing', PEER_KEY_MISSING],
  ['sealRefused', SEAL_REFUSED],
  ['envelopeTooLarge', ENVELOPE_TOO_LARGE],
]

export function errorKind(error: unknown): ErrorKind {
  if (isRelayUnavailable(error)) return 'relayUnavailable'
  if (isRelayNeedsUpdate(error)) return 'relayNeedsUpdate'
  if (isPeerKeyChanged(error)) return 'peerKeyChanged'
  const raw = rawError(error)
  const sealing = SEALING.find(([, code]) => raw.startsWith(code))
  if (sealing) return sealing[0]
  // After "The relay refused it: " (`relay::client::told`)
  if (raw.includes(`${NO_SHARED_GROUP}:`)) return 'noSharedGroup'
  if (raw.includes(`${NEEDS_FIRST_TIME_APPROVAL}:`))
    return 'needsFirstTimeApproval'
  if (raw.includes(`${GROUP_CAPABILITY_NOT_ALLOWED}:`))
    return 'groupCapabilityNotAllowed'
  if (resetsAt(GROUP_DAILY_LIMIT, error) !== null) return 'groupDailyLimit'
  if (dailyLimitResetsAt(error) !== null) return 'dailyLimit'
  return PATTERNS.find(([, pattern]) => pattern.test(raw))?.[0] ?? 'other'
}

/**
 * When a public relay's daily message count resets, for a refusal past it
 * (`daily_limit@<ms>: …`, `services::relay::limits`); `null` otherwise
 */
export function dailyLimitResetsAt(error: unknown): number | null {
  return resetsAt(DAILY_LIMIT, error)
}

/**
 * When the count behind a `<code>@<ms>: …` refusal resets; `null` when
 * the error isn't one. `\b` keeps `daily_limit` from matching inside
 * `group_daily_limit`.
 */
function resetsAt(code: string, error: unknown): number | null {
  const match = new RegExp(`\\b${code}@(\\d+):`).exec(rawError(error))
  return match?.[1] ? Number(match[1]) : null
}

/** The `errors.*` key for the plain sentence */
export function friendlyErrorKey(error: unknown): string {
  return `errors.${errorKind(error)}`
}

/** What the plain sentence names: the reset time of a daily limit */
export function friendlyErrorParams(
  error: unknown,
  locale: string
): Record<string, string> {
  const at = resetsAt(GROUP_DAILY_LIMIT, error) ?? dailyLimitResetsAt(error)
  return at === null ? {} : { time: formatResetTime(at, locale) }
}
