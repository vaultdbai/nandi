/**
 * The relay's reasons for picking a provider (`RouteReason`), as i18n keys
 * and values. The relay sends typed codes with their parameters, never
 * sentences, so every locale words them itself. Pure: no React.
 */
import type { JsonValue, RouteReason } from '@/lib/bindings'

/** One reason, ready for `t(key, values)` */
export interface ReasonText {
  key: string
  values: Record<string, string | number>
}

const CODES: ReadonlySet<RouteReason['code']> = new Set([
  'named',
  'pinned',
  'covers',
  'free',
  'busy',
  'offline',
  'away',
  'usually_quick',
  'dependable',
  'lighter_day',
  'their_turn',
  'trying_again',
])

/**
 * The i18n key and values of `reason`. Times go through `formatDate`
 * (the caller's locale); a reason with an optional part it lacks uses its
 * shorter wording.
 */
export function reasonText(
  reason: RouteReason,
  formatDate: (ms: number) => string
): ReasonText {
  const key = (name: string) => `routing.reason.${name}`
  switch (reason.code) {
    case 'covers':
      return reason.until != null
        ? {
            key: key('coversUntil'),
            values: {
              name: reason.name,
              away: reason.awayName,
              until: formatDate(reason.until),
            },
          }
        : {
            key: key('covers'),
            values: { name: reason.name, away: reason.awayName },
          }
    case 'busy':
      return reason.waitMinutes != null
        ? {
            key: key('busyWait'),
            values: {
              name: reason.name,
              load: reason.load,
              max: reason.max,
              minutes: reason.waitMinutes,
            },
          }
        : {
            key: key('busy'),
            values: { name: reason.name, load: reason.load, max: reason.max },
          }
    case 'away':
      return reason.until != null
        ? {
            key: key('awayUntil'),
            values: { name: reason.name, until: formatDate(reason.until) },
          }
        : { key: key('away'), values: { name: reason.name } }
    default:
      return { key: key(reason.code), values: { name: reason.name } }
  }
}

/** Whether `value` is a reason this app knows how to word */
function isRouteReason(value: JsonValue): boolean {
  return (
    value !== null &&
    typeof value === 'object' &&
    !Array.isArray(value) &&
    typeof value.code === 'string' &&
    CODES.has(value.code as RouteReason['code']) &&
    typeof value.name === 'string'
  )
}

/** The reasons in a `route` task message's payload (unknown ones dropped) */
export function routeReasonsOf(payload: JsonValue): RouteReason[] {
  if (payload === null || typeof payload !== 'object' || Array.isArray(payload))
    return []
  const reasons = payload.reasons
  if (!Array.isArray(reasons)) return []
  // Checked above: a known code and a name
  return reasons.filter(isRouteReason) as unknown as RouteReason[]
}
