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

/** What the owner picks in the availability editor */
export interface AvailabilityChoice {
  status: 'available' | 'busy' | 'away'
  /** The day it ends (local midnight); `undefined`: until they change it */
  until: Date | undefined
  /** The instance ID of who covers; `''`: nobody */
  delegateTo: string
}

/** The editor's starting point: what holds now, a lapsed one as available */
export function choiceFrom(
  availability: Availability | undefined,
  now: number = Date.now()
): AvailabilityChoice {
  const current = currentAvailability(availability, now)
  if (!current) return { status: 'available', until: undefined, delegateTo: '' }
  return {
    status: current.status,
    until: current.until === null ? undefined : new Date(current.until),
    delegateTo: current.delegateTo ?? '',
  }
}

/** What NANDI.md stores: available carries no end and nobody covering */
export function availabilityOf(choice: AvailabilityChoice): Availability {
  if (choice.status === 'available') {
    return { status: 'available', until: null, delegateTo: null }
  }
  const until = choice.until
    ? new Date(
        choice.until.getFullYear(),
        choice.until.getMonth(),
        choice.until.getDate()
      ).getTime()
    : null
  return {
    status: choice.status,
    until,
    delegateTo: choice.delegateTo || null,
  }
}
