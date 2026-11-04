/** The kinds of agents & packs source, with their label keys */
export const MARKETPLACE_KIND_LABELS = {
  public: 'marketplace.registryKind.public',
  community: 'marketplace.registryKind.community',
  personal: 'marketplace.registryKind.personal',
} as const

export type MarketplaceKind = keyof typeof MARKETPLACE_KIND_LABELS

export const isMarketplaceKind = (kind: string): kind is MarketplaceKind =>
  kind in MARKETPLACE_KIND_LABELS
