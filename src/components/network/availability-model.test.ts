import { describe, expect, it } from 'vitest'
import { currentAvailability, formatUntil } from './availability-model'

describe('currentAvailability', () => {
  it('is nothing for someone available or with no availability', () => {
    expect(currentAvailability(undefined)).toBeNull()
    expect(
      currentAvailability({
        status: 'available',
        until: null,
        delegateTo: null,
      })
    ).toBeNull()
  })

  it('keeps busy or away until `until`, then lapses', () => {
    const away = { status: 'away' as const, until: 1000, delegateTo: 'lena-1' }
    expect(currentAvailability(away, 999)).toEqual({
      status: 'away',
      until: 1000,
      delegateTo: 'lena-1',
    })
    expect(currentAvailability(away, 1000)).toBeNull()
    expect(currentAvailability({ status: 'busy' }, 5)).toEqual({
      status: 'busy',
      until: null,
      delegateTo: null,
    })
  })
})

describe('formatUntil', () => {
  it('leaves the time out at midnight', () => {
    const midnight = new Date(2026, 10, 10).getTime()
    expect(formatUntil(midnight, 'en')).not.toMatch(/\d:\d\d/)
    const morning = new Date(2026, 10, 10, 9, 30).getTime()
    expect(formatUntil(morning, 'en')).toMatch(/9:30/)
  })
})
