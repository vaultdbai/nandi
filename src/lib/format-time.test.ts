import { describe, it, expect } from 'vitest'
import { formatDuration, formatRelativeTime } from './format-time'

const NOW = new Date('2026-10-02T15:00:00')
const hoursAgo = (h: number) => NOW.getTime() - h * 3_600_000

describe('format-time', () => {
  it('formats relative times from ms timestamps or ISO strings', () => {
    expect(formatRelativeTime(hoursAgo(2), 'en', NOW)).toBe('2 hr. ago')
    expect(
      formatRelativeTime(new Date(hoursAgo(0.001)).toISOString(), 'en', NOW)
    ).toBe('now')
  })

  it('formats durations', () => {
    expect(formatDuration(130_000)).toBe('2m 10s')
    expect(formatDuration(120_000)).toBe('2m')
    expect(formatDuration(45_000)).toBe('45s')
    expect(formatDuration(3_900_000)).toBe('1h 5m')
  })
})
