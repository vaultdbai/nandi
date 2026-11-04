/**
 * Device-local "don't show this again" for hints (Get started, the backup
 * nudge). Lives in localStorage: losing it only brings a hint back.
 */
const PREFIX = 'nandi.dismissed.'

export function isDismissed(key: string, now = Date.now()): boolean {
  try {
    const raw = localStorage.getItem(PREFIX + key)
    if (raw === null) return false
    const until = Number(raw)
    // `0` (or anything unreadable) means for good
    return !Number.isFinite(until) || until === 0 || until > now
  } catch {
    return false
  }
}

/** Hides `key` for `forMs`, or for good when it's omitted */
export function dismiss(key: string, forMs?: number, now = Date.now()) {
  try {
    localStorage.setItem(PREFIX + key, String(forMs ? now + forMs : 0))
  } catch {
    // Storage unavailable: the hint just comes back next time
  }
}
