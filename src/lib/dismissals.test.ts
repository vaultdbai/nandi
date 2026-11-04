import { beforeEach, describe, expect, it } from 'vitest'
import { dismiss, isDismissed } from './dismissals'

describe('dismissals', () => {
  beforeEach(() => localStorage.clear())

  it('hides a hint for good', () => {
    expect(isDismissed('hint')).toBe(false)
    dismiss('hint')
    expect(isDismissed('hint', Date.now() + 1e12)).toBe(true)
  })

  it('brings a snoozed hint back when the snooze ends', () => {
    dismiss('hint', 1000, 0)
    expect(isDismissed('hint', 999)).toBe(true)
    expect(isDismissed('hint', 1001)).toBe(false)
  })
})
