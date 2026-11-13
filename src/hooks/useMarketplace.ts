import { useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { MarketplaceFilter } from '@/lib/bindings'
import { logger } from '@/lib/logger'
import { marketplaceFilter } from '@/lib/marketplace-filter'
import type { Result } from '@/lib/tauri-bindings'

/** Everything marketplace-related lives under `['marketplace']` */
export const marketplaceQueryKeys = {
  all: ['marketplace'] as const,
  packs: (filter: Partial<MarketplaceFilter> = {}) =>
    ['marketplace', 'packs', filter] as const,
  agents: (filter: Partial<MarketplaceFilter> = {}) =>
    ['marketplace', 'agents', filter] as const,
  detail: (agentId: string) => ['marketplace', 'detail', agentId] as const,
  installed: ['marketplace', 'installed'] as const,
  consent: (agentId: string) => ['marketplace', 'consent', agentId] as const,
  registries: ['marketplace', 'registries'] as const,
}

async function unwrap<T>(what: string, result: Promise<Result<T, string>>) {
  const resolved = await result
  if (resolved.status === 'error') {
    logger.error(`Failed to load ${what}`, { error: resolved.error })
    throw new Error(resolved.error)
  }
  return resolved.data
}

/** Registries are cached by the core (ETag + TTL), so this is cheap */
const STALE_MS = 60_000

export function useMarketplacePacks(filter: Partial<MarketplaceFilter> = {}) {
  return useQuery({
    queryKey: marketplaceQueryKeys.packs(filter),
    queryFn: () =>
      unwrap(
        'marketplace packs',
        client.listMarketplacePacks(marketplaceFilter(filter))
      ),
    staleTime: STALE_MS,
  })
}

export function useMarketplaceAgents(filter: Partial<MarketplaceFilter> = {}) {
  return useQuery({
    queryKey: marketplaceQueryKeys.agents(filter),
    queryFn: () =>
      unwrap(
        'marketplace agents',
        client.listAvailableAgents(marketplaceFilter(filter))
      ),
    staleTime: STALE_MS,
  })
}

/** Whether a marketplace agent may run yet (first-use consent) */
export function useAgentConsent(agentId: string, enabled: boolean) {
  return useQuery({
    queryKey: marketplaceQueryKeys.consent(agentId),
    queryFn: () => unwrap('consent', client.getAgentConsent(agentId)),
    enabled,
  })
}

/** Configured registries, with their counts and freshness */
export function useRegistries() {
  return useQuery({
    queryKey: marketplaceQueryKeys.registries,
    queryFn: () => unwrap('registries', client.listRegistries()),
  })
}
