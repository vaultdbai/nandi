/**
 * Declared capabilities and agent cards (TanStack Query). The core keeps
 * the capability index current from repo events; these queries refetch
 * when invalidated under `capabilityQueryKeys.all`.
 */
import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { CapabilityDefinition, CapabilityOwner } from '@/lib/bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

/** Everything capability-related lives under `['capabilities']` */
export const capabilityQueryKeys = {
  all: ['capabilities'] as const,
  list: ['capabilities', 'list'] as const,
  agents: (capability: string) =>
    ['capabilities', 'agents', capability] as const,
  card: (agentId: string) => ['capabilities', 'card', agentId] as const,
  // Definitions, providers, evidence
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
  return useClientQuery(capabilityQueryKeys.list, () =>
    client.listDeclaredCapabilities()
  )
}

/** Cards of the local agents serving `capability` (idle while `null`) */
export function useAgentsByCapability(capability: string | null) {
  const name = capability?.trim() ?? ''
  return useClientQuery(
    capabilityQueryKeys.agents(name),
    () => client.findAgentsByCapability(name),
    { enabled: name !== '' }
  )
}

/** The card other instances see for a local agent (idle while `null`) */
export function useAgentCard(agentId: string | null) {
  return useClientQuery(
    capabilityQueryKeys.card(agentId ?? ''),
    () => client.getAgentCard(agentId ?? ''),
    { enabled: !!agentId }
  )
}

// Capability definitions (`.nandi/capabilities/`, registries, and the
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
  return useClientQuery(capabilityQueryKeys.listings, () =>
    client.listCapabilities()
  )
}

/** One definition (idle while `null`) */
export function useCapability(id: string | null) {
  return useClientQuery(
    capabilityQueryKeys.definition(id ?? ''),
    () => client.getCapability(id ?? ''),
    { enabled: !!id }
  )
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
  return useClientMutation(
    (definition: CapabilityDefinition) => client.saveCapability(definition),
    { onSuccess: store }
  )
}

/** Hands the capability to `newOwner`; the file keeps the history */
export function useTransferCapabilityOwnership() {
  const store = useStoreDefinition()
  return useClientMutation(
    (vars: { id: string; newOwner: CapabilityOwner; reason: string | null }) =>
      client.transferCapabilityOwnership(vars.id, vars.newOwner, vars.reason),
    { onSuccess: store }
  )
}
