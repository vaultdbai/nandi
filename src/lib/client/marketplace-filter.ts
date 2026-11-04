import type { MarketplaceFilter } from '@/lib/bindings'

/** A full filter from the fields given (the rest off / empty). */
export function marketplaceFilter(
  filter: Partial<MarketplaceFilter> = {}
): MarketplaceFilter {
  return {
    query: filter.query ?? null,
    tags: filter.tags ?? [],
    verifiedOnly: filter.verifiedOnly ?? false,
    installedOnly: filter.installedOnly ?? false,
    refresh: filter.refresh ?? false,
  }
}

/** The filter as `/api/marketplace/*` query parameters. */
export function marketplaceQuery(filter: Partial<MarketplaceFilter> = {}) {
  const full = marketplaceFilter(filter)
  const params = new URLSearchParams()
  if (full.query) params.set('query', full.query)
  if (full.tags.length > 0) params.set('tags', full.tags.join(','))
  if (full.verifiedOnly) params.set('verifiedOnly', 'true')
  if (full.installedOnly) params.set('installedOnly', 'true')
  if (full.refresh) params.set('refresh', 'true')
  const query = params.toString()
  return query ? `?${query}` : ''
}
