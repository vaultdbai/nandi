import { describe, expect, it } from 'vitest'
import {
  availabilityOf,
  choiceFrom,
  currentAvailability,
  formatUntil,
} from './availability-model'

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

describe('choiceFrom and availabilityOf', () => {
  it('starts a lapsed or missing availability as available', () => {
    expect(choiceFrom(undefined)).toEqual({
      status: 'available',
      until: undefined,
      delegateTo: '',
    })
    const lapsed = {
      status: 'away' as const,
      until: 1000,
      delegateTo: 'lena-1',
    }
    expect(choiceFrom(lapsed, 2000).status).toBe('available')
  })

  it('keeps what holds now', () => {
    const away = { status: 'away' as const, until: 5000, delegateTo: 'lena-1' }
    expect(choiceFrom(away, 1000)).toEqual({
      status: 'away',
      until: new Date(5000),
      delegateTo: 'lena-1',
    })
  })

  it('stores available with no end and nobody covering', () => {
    expect(
      availabilityOf({
        status: 'available',
        until: new Date(2026, 10, 10),
        delegateTo: 'lena-1',
      })
    ).toEqual({ status: 'available', until: null, delegateTo: null })
  })

  it('stores the end as that day at local midnight', () => {
    expect(
      availabilityOf({
        status: 'away',
        until: new Date(2026, 10, 10, 15, 30),
        delegateTo: '',
      })
    ).toEqual({
      status: 'away',
      until: new Date(2026, 10, 10).getTime(),
      delegateTo: null,
    })
    expect(
      availabilityOf({ status: 'busy', until: undefined, delegateTo: 'lena-1' })
    ).toEqual({ status: 'busy', until: null, delegateTo: 'lena-1' })
  })
})
