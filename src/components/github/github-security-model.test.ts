import { describe, expect, it } from 'vitest'
import { securityLine } from './github-security-model'

const NOW = new Date('2026-11-20T12:00:00Z')

describe('securityLine', () => {
  it('says when it never ran', () => {
    expect(securityLine(undefined, 'en', NOW)).toEqual({
      key: 'github.security.never',
      values: {},
    })
    expect(securityLine({ checkedAt: null, findings: 0 }, 'en', NOW).key).toBe(
      'github.security.never'
    )
  })

  it('says when it last ran and what it found', () => {
    const twoHoursAgo = NOW.getTime() - 2 * 60 * 60 * 1000
    const clean = securityLine(
      { checkedAt: twoHoursAgo, findings: 0 },
      'en',
      NOW
    )
    expect(clean.key).toBe('github.security.clean')
    expect(clean.values.when).toMatch(/2 hr/)

    const found = securityLine(
      { checkedAt: twoHoursAgo, findings: 3 },
      'en',
      NOW
    )
    expect(found).toEqual({
      key: 'github.security.found',
      values: { when: clean.values.when, count: 3 },
    })
  })
})
