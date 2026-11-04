import { describe, expect, it } from 'vitest'
import {
  allTags,
  cardFaces,
  EMPTY_FILTERS,
  filterBreakdown,
  filterListings,
  formatDuration,
  formatRate,
  groupBySection,
  sectionsOf,
  sortBreakdown,
  totalEvidence,
} from './capability-model'
import { evidence, listing, providerStats } from './capability-fixtures'

describe('sectionsOf', () => {
  it('puts a listing in every relationship it has', () => {
    expect(sectionsOf(listing('a', { owned: true, provided: true }))).toEqual([
      'owned',
      'provided',
    ])
    expect(sectionsOf(listing('b', { used: true }))).toEqual(['used'])
  })

  it('sends listings with no relationship to Discover', () => {
    expect(sectionsOf(listing('c'))).toEqual(['discover'])
  })
})

describe('cardFaces', () => {
  it('shows both faces when provided and used', () => {
    expect(cardFaces(listing('a', { provided: true, used: true }))).toEqual({
      provider: true,
      consumer: true,
    })
  })

  it('shows the consumer face for something to discover', () => {
    expect(cardFaces(listing('a'))).toEqual({
      provider: false,
      consumer: true,
    })
  })

  it('shows neither for one you own but nobody provides or uses', () => {
    expect(cardFaces(listing('a', { owned: true }))).toEqual({
      provider: false,
      consumer: false,
    })
  })
})

describe('filterListings', () => {
  const listings = [
    listing('code-review.python', { owned: true }, { tags: ['python'] }),
    listing(
      'summarize-paper',
      {},
      {
        tags: ['research'],
        policy: {
          visibility: 'team',
          maxConcurrentPerProvider: 1,
          totalMaxConcurrent: 0,
          slaSeconds: 0,
          approvalRequired: false,
          cost: null,
          autoReassign: 'never',
        },
      }
    ),
  ]

  it('searches ID, name, description, tags, and owner', () => {
    const ids = (query: string) =>
      filterListings(listings, { ...EMPTY_FILTERS, query }).map(
        l => l.definition.id
      )
    expect(ids('PYTHON')).toEqual(['code-review.python'])
    expect(ids('research')).toEqual(['summarize-paper'])
    expect(ids('Does summarize')).toEqual(['summarize-paper'])
    expect(ids('')).toHaveLength(2)
  })

  it('filters by visibility, tag, and ownership', () => {
    const ids = (filters: Partial<typeof EMPTY_FILTERS>) =>
      filterListings(listings, { ...EMPTY_FILTERS, ...filters }).map(
        l => l.definition.id
      )
    expect(ids({ visibility: 'team' })).toEqual(['summarize-paper'])
    expect(ids({ tag: 'python' })).toEqual(['code-review.python'])
    expect(ids({ ownership: 'mine' })).toEqual(['code-review.python'])
    expect(ids({ ownership: 'others' })).toEqual(['summarize-paper'])
  })

  it('reads an unset policy as private', () => {
    const bare = listing('bare', {}, { policy: undefined })
    expect(
      filterListings([bare], { ...EMPTY_FILTERS, visibility: 'private' })
    ).toHaveLength(1)
  })

  it('collects tags without repeats', () => {
    expect(allTags([...listings, ...listings])).toEqual(['python', 'research'])
  })
})

describe('groupBySection', () => {
  it('sorts each section by name', () => {
    const groups = groupBySection([
      listing('zeta', { owned: true }),
      listing('alpha', { owned: true, used: true }),
      listing('mid'),
    ])
    expect(groups.owned.map(l => l.definition.id)).toEqual(['alpha', 'zeta'])
    expect(groups.used.map(l => l.definition.id)).toEqual(['alpha'])
    expect(groups.discover.map(l => l.definition.id)).toEqual(['mid'])
    expect(groups.provided).toEqual([])
  })
})

describe('totalEvidence', () => {
  it('sums providers and weights p50 by invocations', () => {
    const totals = totalEvidence([
      evidence({
        count: 3,
        successCount: 3,
        p50Ms: 1000,
        p95Ms: 2000,
        costUsdTotal: 0.5,
      }),
      evidence({
        count: 1,
        successCount: 0,
        failedCount: 1,
        p50Ms: 5000,
        p95Ms: 9000,
        costUsdTotal: null,
      }),
    ])
    expect(totals.count).toBe(4)
    expect(totals.successRate).toBe(0.75)
    expect(totals.p50Ms).toBe(2000)
    expect(totals.p95Ms).toBe(9000)
    expect(totals.costUsd).toBe(0.5)
  })

  it('reports unknowns as null without evidence', () => {
    const totals = totalEvidence([])
    expect(totals).toMatchObject({
      count: 0,
      successRate: null,
      p50Ms: null,
      costUsd: null,
    })
  })
})

describe('provider breakdown', () => {
  const rows = [
    providerStats('fast', { evidence: { count: 10, p50Ms: 100 } }),
    providerStats('slow', { evidence: { count: 2, p50Ms: 900 } }),
    providerStats('new', { evidence: { count: 0, p50Ms: null } }),
  ]

  it('sorts by a column, unknown values last either way', () => {
    const ids = (direction: 'asc' | 'desc') =>
      sortBreakdown(rows, { key: 'p50', direction }).map(
        r => r.provider.providerId
      )
    expect(ids('asc')).toEqual(['fast', 'slow', 'new'])
    expect(ids('desc')).toEqual(['slow', 'fast', 'new'])
  })

  it('filters by provider or instance', () => {
    expect(
      filterBreakdown(rows, 'slo').map(r => r.provider.providerId)
    ).toEqual(['slow'])
    expect(filterBreakdown(rows, 'i-me')).toHaveLength(3)
  })
})

describe('formatting', () => {
  it('formats durations and rates', () => {
    expect(formatDuration(null)).toBe('—')
    expect(formatDuration(420)).toBe('420 ms')
    expect(formatDuration(4200)).toBe('4.2 s')
    expect(formatDuration(120_000)).toBe('2.0 min')
    expect(formatRate(0.756)).toBe('76%')
    expect(formatRate(null)).toBe('—')
  })
})
