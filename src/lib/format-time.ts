const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ['day', 24 * 60 * 60 * 1000],
  ['hour', 60 * 60 * 1000],
  ['minute', 60 * 1000],
]

/** "2 min ago" / "il y a 2 min", localized. Under a minute reads as "now". */
export function formatRelativeTime(
  time: string | number,
  locale: string,
  now: Date = new Date()
): string {
  const diff = new Date(time).getTime() - now.getTime()
  const format = new Intl.RelativeTimeFormat(locale, {
    numeric: 'auto',
    style: 'short',
  })
  for (const [unit, ms] of UNITS) {
    if (Math.abs(diff) >= ms) return format.format(Math.round(diff / ms), unit)
  }
  return format.format(0, 'second')
}

/** "2m 10s" style durations */
export function formatDuration(ms: number): string {
  const totalSeconds = Math.round(ms / 1000)
  const hours = Math.floor(totalSeconds / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)
  const seconds = totalSeconds % 60
  if (hours > 0) return `${hours}h ${minutes}m`
  if (minutes > 0)
    return seconds > 0 ? `${minutes}m ${seconds}s` : `${minutes}m`
  return `${seconds}s`
}

/**
 * When a daily count resets (00:00 UTC), in the user's own clock: "Wed
 * 01:00" / "mer. 01:00"
 */
export function formatResetTime(time: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, {
    weekday: 'short',
    hour: 'numeric',
    minute: '2-digit',
  }).format(new Date(time))
}
