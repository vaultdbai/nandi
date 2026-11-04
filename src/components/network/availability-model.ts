/**
 * A member's availability as their NANDI.md sets it (`availability:` on
 * their gateway card): busy or away, until when, and who covers.
 */
import type { Availability } from '@/lib/bindings'

/** What the profile shows; `null` when they're available */
export interface AvailabilityView {
  status: 'busy' | 'away'
  /** Epoch ms; `null`: until they change it */
  until: number | null
  /** The instance ID of who covers */
  delegateTo: string | null
}

/** What holds at `now`: nothing once `until` has passed */
export function currentAvailability(
  availability: Availability | undefined,
  now: number = Date.now()
): AvailabilityView | null {
  const status = availability?.status ?? 'available'
  if (status === 'available') return null
  const until = availability?.until ?? null
  if (until !== null && until <= now) return null
  return { status, until, delegateTo: availability?.delegateTo ?? null }
}

/** `until` as a date, with the time unless it's midnight */
export function formatUntil(until: number, language: string): string {
  const at = new Date(until)
  const midnight =
    at.getHours() === 0 && at.getMinutes() === 0 && at.getSeconds() === 0
  return new Intl.DateTimeFormat(language, {
    weekday: 'short',
    month: 'short',
    day: 'numeric',
    ...(midnight ? {} : { hour: 'numeric', minute: '2-digit' }),
  }).format(at)
}
