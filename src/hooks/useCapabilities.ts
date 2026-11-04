/**
 * Declared capabilities and agent cards (TanStack Query). The core keeps
 * the capability index current from repo events; these queries refetch
 * when invalidated under `capabilityQueryKeys.all`.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type {
  AgentCard,
  CapabilityDefinition,
  CapabilityListing,
  CapabilityOwner,
  CapabilityProvider,
} from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

/** Everything capability-related lives under `['capabilities']` */
export const capabilityQueryKeys = {
  all: ['capabilities'] as const,
  list: ['capabilities', 'list'] as const,
  agents: (capability: string) =>
    ['capabilities', 'agents', capability] as const,
  card: (agentId: string) => ['capabilities', 'card', agentId] as const,
  // Wave A: definitions, providers, evidence
  listings: ['capabilities', 'listings'] as const,
  definition: (id: string) => ['capabilities', 'definition', id] as const,
  providers: (id: string) => ['capabilities', 'providers', id] as const,
  evidence: (id: string, windowDays: number) =>
    ['capabilities', 'evidence', id, windowDays] as const,
  breakdown: (id: string) => ['capabilities', 'breakdown', id] as const,
  ambiguities: (id: string) => ['capabilities', 'ambiguities', id] as const,
}

/** Every capability a local agent serves, sorted */
export function useCapabilities() {
  return useQuery({
    queryKey: capabilityQueryKeys.list,
    queryFn: async (): Promise<string[]> =>
      unwrapResult(await client.listDeclaredCapabilities()),
  })
}

/** Cards of the local agents serving `capability` (idle while `null`) */
export function useAgentsByCapability(capability: string | null) {
  const name = capability?.trim() ?? ''
  return useQuery({
    queryKey: capabilityQueryKeys.agents(name),
    queryFn: async (): Promise<AgentCard[]> =>
      unwrapResult(await client.findAgentsByCapability(name)),
    enabled: name !== '',
  })
}

/** The card other instances see for a local agent (idle while `null`) */
export function useAgentCard(agentId: string | null) {
  return useQuery({
    queryKey: capabilityQueryKeys.card(agentId ?? ''),
    queryFn: async (): Promise<AgentCard> =>
      unwrapResult(await client.getAgentCard(agentId ?? '')),
    enabled: !!agentId,
  })
}

// Wave A: capability definitions (`.nandi/capabilities/`, registries, and the
// network), their providers, and ownership.

/** Core events after which capability queries may be stale */
const CAPABILITY_EVENT_PREFIXES = [
  'capability://',
  'provider://',
  'evidence://',
  'registry://',
  'tool://catalog_changed',
]

/** Whether a core event should refetch the capability queries */
export function isCapabilityEvent(name: string): boolean {
  return CAPABILITY_EVENT_PREFIXES.some(prefix => name.startsWith(prefix))
}

/**
 * Refetches everything under `capabilityQueryKeys.all` on capability,
 * provider, evidence, registry, and tool-catalog events. Mount once per
 * view, not per card.
 */
export function useCapabilityEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (!isCapabilityEvent(event.event)) return
      void queryClient.invalidateQueries({
        queryKey: capabilityQueryKeys.all,
      })
    })
    return () => subscription.unsubscribe()
  }, [queryClient])
}

/** Every capability with this instance's relation (Capabilities page) */
export function useCapabilityListings() {
  return useQuery({
    queryKey: capabilityQueryKeys.listings,
    queryFn: async (): Promise<CapabilityListing[]> =>
      unwrapResult(await client.listCapabilities()),
  })
}

/** One definition (idle while `null`) */
export function useCapability(id: string | null) {
  return useQuery({
    queryKey: capabilityQueryKeys.definition(id ?? ''),
    queryFn: async (): Promise<CapabilityDefinition> =>
      unwrapResult(await client.getCapability(id ?? '')),
    enabled: !!id,
  })
}

/** Agents and workflows providing `id`, here and on the network */
export function useCapabilityProviders(id: string | null) {
  return useQuery({
    queryKey: capabilityQueryKeys.providers(id ?? ''),
    queryFn: async (): Promise<CapabilityProvider[]> =>
      unwrapResult(await client.listProvidersForCapability(id ?? '')),
    enabled: !!id,
  })
}

/** Caches a saved definition and marks the listings stale */
function useStoreDefinition() {
  const queryClient = useQueryClient()
  return (definition: CapabilityDefinition) => {
    queryClient.setQueryData(
      capabilityQueryKeys.definition(definition.id),
      definition
    )
    void queryClient.invalidateQueries({
      queryKey: capabilityQueryKeys.listings,
    })
  }
}

/** Creates or replaces the definition's file; resolves with it as saved */
export function useSaveCapability() {
  const store = useStoreDefinition()
  return useMutation({
    mutationFn: async (
      definition: CapabilityDefinition
    ): Promise<CapabilityDefinition> =>
      unwrapResult(await client.saveCapability(definition)),
    onSuccess: store,
  })
}

export function useDeleteCapability() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (id: string): Promise<null> =>
      unwrapResult(await client.deleteCapability(id)),
    onSuccess: (_, id) => {
      queryClient.removeQueries({
        queryKey: capabilityQueryKeys.definition(id),
      })
      void queryClient.invalidateQueries({
        queryKey: capabilityQueryKeys.listings,
      })
    },
  })
}

/** Hands the capability to `newOwner`; the file keeps the history */
export function useTransferCapabilityOwnership() {
  const store = useStoreDefinition()
  return useMutation({
    mutationFn: async (vars: {
      id: string
      newOwner: CapabilityOwner
      reason: string | null
    }): Promise<CapabilityDefinition> =>
      unwrapResult(
        await client.transferCapabilityOwnership(
          vars.id,
          vars.newOwner,
          vars.reason
        )
      ),
    onSuccess: store,
  })
}
