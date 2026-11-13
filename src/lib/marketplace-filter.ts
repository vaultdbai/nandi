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
