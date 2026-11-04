import { describe, expect, it } from 'vitest'
import { formatCostUsd, formatTokenCount } from './format-usage'

describe('formatTokenCount', () => {
  it('compacts large counts', () => {
    expect(formatTokenCount(12_400, 'en')).toBe('12.4K')
    expect(formatTokenCount(3_100, 'en')).toBe('3.1K')
    expect(formatTokenCount(950, 'en')).toBe('950')
  })
})

describe('formatCostUsd', () => {
  it('keeps two significant digits below a cent', () => {
    expect(formatCostUsd(0.0042, 'en')).toBe('$0.0042')
    expect(formatCostUsd(0.004219, 'en')).toBe('$0.0042')
  })

  it('uses cents otherwise', () => {
    expect(formatCostUsd(1.234, 'en')).toBe('$1.23')
    expect(formatCostUsd(0, 'en')).toBe('$0.00')
  })
})
