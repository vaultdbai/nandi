/**
 * Pure rules for the Capabilities page: which sections a listing belongs
 * to, which faces its card shows, filtering, evidence totals, and the
 * provider table's sort.
 */
import type {
  CapabilityEvidence,
  CapabilityListing,
  CapabilityProviderStats,
  Visibility,
} from '@/lib/bindings'

/** The page's relationship sections, in order */
export const CAPABILITY_SECTIONS = [
  'owned',
  'provided',
  'used',
  'discover',
] as const
export type CapabilitySection = (typeof CAPABILITY_SECTIONS)[number]

/** Owned, provided, and used can overlap; Discover is everything else */
export function sectionsOf(listing: CapabilityListing): CapabilitySection[] {
  const sections: CapabilitySection[] = []
  if (listing.owned) sections.push('owned')
  if (listing.provided) sections.push('provided')
  if (listing.used) sections.push('used')
  if (sections.length === 0) sections.push('discover')
  return sections
}

export interface CardFaces {
  /** This instance provides it: online, load, success rate */
  provider: boolean
  /** This instance uses it or could: providers online, evidence, Delegate */
  consumer: boolean
}

/** Neither face: a compact card with owner, description, and Details */
export function cardFaces(listing: CapabilityListing): CardFaces {
  return {
    provider: listing.provided,
    consumer: listing.used || (!listing.owned && !listing.provided),
  }
}

/** Unset policy reads as private (the core's default) */
export function visibilityOf(listing: CapabilityListing): Visibility {
  return listing.definition.policy?.visibility ?? 'private'
}

export type OwnershipFilter = 'all' | 'mine' | 'others'

export interface CapabilityFilters {
  query: string
  visibility: Visibility | 'all'
  /** `null`: any tag */
  tag: string | null
  ownership: OwnershipFilter
}

export const EMPTY_FILTERS: CapabilityFilters = {
  query: '',
  visibility: 'all',
  tag: null,
  ownership: 'all',
}

/** Matches ID, name, description, tags, and owner, case-insensitively */
export function matchesQuery(listing: CapabilityListing, query: string) {
  const needle = query.trim().toLowerCase()
  if (needle === '') return true
  const { definition } = listing
  return [
    definition.id,
    definition.name,
    definition.description,
    definition.owner.displayName,
    definition.owner.id,
    ...(definition.tags ?? []),
  ].some(text => text.toLowerCase().includes(needle))
}

export function filterListings(
  listings: CapabilityListing[],
  filters: CapabilityFilters
): CapabilityListing[] {
  return listings.filter(listing => {
    if (!matchesQuery(listing, filters.query)) return false
    if (
      filters.visibility !== 'all' &&
      visibilityOf(listing) !== filters.visibility
    ) {
      return false
    }
    if (filters.tag && !(listing.definition.tags ?? []).includes(filters.tag)) {
      return false
    }
    if (filters.ownership === 'mine' && !listing.owned) return false
    if (filters.ownership === 'others' && listing.owned) return false
    return true
  })
}

/** Each section's listings, sorted by name */
export function groupBySection(
  listings: CapabilityListing[]
): Record<CapabilitySection, CapabilityListing[]> {
  const groups: Record<CapabilitySection, CapabilityListing[]> = {
    owned: [],
    provided: [],
    used: [],
    discover: [],
  }
  const sorted = [...listings].sort((a, b) =>
    (a.definition.name || a.definition.id).localeCompare(
      b.definition.name || b.definition.id
    )
  )
  for (const listing of sorted) {
    for (const section of sectionsOf(listing)) groups[section].push(listing)
  }
  return groups
}

/** Every tag used by `listings`, sorted, no repeats */
export function allTags(listings: CapabilityListing[]): string[] {
  const tags = new Set<string>()
  for (const listing of listings) {
    for (const tag of listing.definition.tags ?? []) tags.add(tag)
  }
  return [...tags].sort((a, b) => a.localeCompare(b))
}

export interface EvidenceTotals {
  count: number
  successCount: number
  failedCount: number
  timeoutCount: number
  /** `null` with no invocations */
  successRate: number | null
  /** Invocation-weighted mean of the providers' p50s */
  p50Ms: number | null
  /** The worst provider's p95 */
  p95Ms: number | null
  /** `null` when no provider reports cost */
  costUsd: number | null
  tokensIn: number
  tokensOut: number
}

/** Sums per-provider evidence into the capability's totals */
export function totalEvidence(rows: CapabilityEvidence[]): EvidenceTotals {
  let count = 0
  let successCount = 0
  let failedCount = 0
  let timeoutCount = 0
  let tokensIn = 0
  let tokensOut = 0
  let cost: number | null = null
  let p50Weight = 0
  let p50Sum = 0
  let p95: number | null = null
  for (const row of rows) {
    count += row.count
    successCount += row.successCount
    failedCount += row.failedCount
    timeoutCount += row.timeoutCount
    tokensIn += row.tokensInTotal
    tokensOut += row.tokensOutTotal
    if (row.costUsdTotal !== null) cost = (cost ?? 0) + row.costUsdTotal
    if (row.p50Ms !== null && row.count > 0) {
      p50Sum += row.p50Ms * row.count
      p50Weight += row.count
    }
    if (row.p95Ms !== null) p95 = Math.max(p95 ?? 0, row.p95Ms)
  }
  return {
    count,
    successCount,
    failedCount,
    timeoutCount,
    successRate: count > 0 ? successCount / count : null,
    p50Ms: p50Weight > 0 ? p50Sum / p50Weight : null,
    p95Ms: p95,
    costUsd: cost,
    tokensIn,
    tokensOut,
  }
}

/** Success over invocations; `null` with none */
export function successRateOf(row: CapabilityEvidence): number | null {
  return row.count > 0 ? row.successCount / row.count : null
}

export type BreakdownSortKey =
  | 'provider'
  | 'kind'
  | 'status'
  | 'load'
  | 'successRate'
  | 'p50'
  | 'p95'
  | 'calls'
  | 'cost'

export interface BreakdownSort {
  key: BreakdownSortKey
  direction: 'asc' | 'desc'
}

export function providerLabel(row: CapabilityProviderStats): string {
  return row.provider.displayName ?? row.provider.providerId
}

function sortValue(
  row: CapabilityProviderStats,
  key: BreakdownSortKey
): string | number | null {
  switch (key) {
    case 'provider':
      return `${row.provider.instanceId}/${providerLabel(row)}`.toLowerCase()
    case 'kind':
      return row.provider.providerKind
    case 'status':
      return row.online ? 1 : 0
    case 'load':
      return row.load
    case 'successRate':
      return successRateOf(row.evidence)
    case 'p50':
      return row.evidence.p50Ms
    case 'p95':
      return row.evidence.p95Ms
    case 'calls':
      return row.evidence.count
    case 'cost':
      return row.evidence.costUsdTotal
  }
}

/** Sorted copy; unknown values (`null`) always go last */
export function sortBreakdown(
  rows: CapabilityProviderStats[],
  sort: BreakdownSort
): CapabilityProviderStats[] {
  const sign = sort.direction === 'asc' ? 1 : -1
  return [...rows].sort((a, b) => {
    const left = sortValue(a, sort.key)
    const right = sortValue(b, sort.key)
    if (left === null && right === null) return 0
    if (left === null) return 1
    if (right === null) return -1
    if (left < right) return -sign
    if (left > right) return sign
    return 0
  })
}

/** Matches instance, provider ID, or display name */
export function filterBreakdown(
  rows: CapabilityProviderStats[],
  query: string
): CapabilityProviderStats[] {
  const needle = query.trim().toLowerCase()
  if (needle === '') return rows
  return rows.filter(row =>
    [row.provider.instanceId, row.provider.providerId, providerLabel(row)].some(
      text => text.toLowerCase().includes(needle)
    )
  )
}

/** "—" for unknown; ms under a second, else seconds, else minutes */
export function formatDuration(ms: number | null): string {
  if (ms === null) return '—'
  if (ms < 1000) return `${Math.round(ms)} ms`
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)} s`
  return `${(ms / 60_000).toFixed(1)} min`
}

export function formatRate(rate: number | null): string {
  return rate === null ? '—' : `${Math.round(rate * 100)}%`
}

export function formatUsd(usd: number | null): string {
  if (usd === null) return '—'
  return `$${usd < 1 ? usd.toFixed(4) : usd.toFixed(2)}`
}
