/**
 * Evidence, provider breakdown, and ambiguity clusters for a capability
 * (TanStack Query). Keys live under `capabilityQueryKeys.all`, so the
 * views' event wiring (`useCapabilityEvents`) refetches them too.
 */
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { AmbiguityCluster } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'
import { capabilityQueryKeys } from './useCapabilities'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

/** The window evidence is shown over unless a caller asks otherwise */
export const DEFAULT_EVIDENCE_WINDOW_DAYS = 30

/** Per-provider evidence over `windowDays` (idle while `null`) */
export function useCapabilityEvidence(
  capabilityId: string | null,
  windowDays: number = DEFAULT_EVIDENCE_WINDOW_DAYS
) {
  const id = capabilityId ?? ''
  return useClientQuery(
    capabilityQueryKeys.evidence(id, windowDays),
    () => client.getCapabilityEvidence(id, windowDays),
    { enabled: id !== '' }
  )
}

/** One row per provider: who, online, load, evidence (idle while `null`) */
export function useCapabilityProviderBreakdown(capabilityId: string | null) {
  const id = capabilityId ?? ''
  return useClientQuery(
    capabilityQueryKeys.breakdown(id),
    () => client.getCapabilityProviderBreakdown(id),
    { enabled: id !== '' }
  )
}

/** Clarification clusters, most frequent first (idle while `null`) */
export function useCapabilityAmbiguities(capabilityId: string | null) {
  const id = capabilityId ?? ''
  return useQuery({
    queryKey: capabilityQueryKeys.ambiguities(id),
    queryFn: async (): Promise<AmbiguityCluster[]> => {
      const clusters = unwrapResult(await client.listCapabilityAmbiguities(id))
      return [...clusters].sort((a, b) => b.occurrenceCount - a.occurrenceCount)
    },
    enabled: id !== '',
  })
}

/** Adds a cluster to the input schema; resolves with the saved definition */
export function usePromoteAmbiguity() {
  const queryClient = useQueryClient()
  return useClientMutation(
    (vars: { capabilityId: string; clusterId: string }) =>
      client.promoteAmbiguityToSchema(vars.capabilityId, vars.clusterId),
    {
      onSuccess: (definition, vars) => {
        queryClient.setQueryData(
          capabilityQueryKeys.definition(vars.capabilityId),
          definition
        )
        void queryClient.invalidateQueries({
          queryKey: capabilityQueryKeys.ambiguities(vars.capabilityId),
        })
        void queryClient.invalidateQueries({
          queryKey: capabilityQueryKeys.listings,
        })
      },
    }
  )
}
