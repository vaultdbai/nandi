/**
 * Subscribed capability registries (TanStack Query, refetched on
 * `registry://synced` / `registry://sync_failed`). A sync changes which
 * capabilities exist, so it also invalidates everything under
 * `capabilityQueryKeys.all`.
 */
import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { RegistryKind } from '@/lib/bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'
import { capabilityQueryKeys } from './useCapabilities'

export const capabilityRegistryQueryKeys = {
  all: ['capability-registries'] as const,
}

export interface AddCapabilityRegistryInput {
  url: string
  /** `null` detects it from the URL */
  kind: RegistryKind | null
  branchPrefix: string | null
  userSlug: string | null
}

export function useCapabilityRegistries() {
  const queryClient = useQueryClient()

  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (
        event.event === 'registry://synced' ||
        event.event === 'registry://sync_failed'
      ) {
        void queryClient.invalidateQueries({
          queryKey: capabilityRegistryQueryKeys.all,
        })
        void queryClient.invalidateQueries({
          queryKey: capabilityQueryKeys.all,
        })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return useClientQuery(capabilityRegistryQueryKeys.all, () =>
    client.listCapabilityRegistries()
  )
}

function useInvalidate() {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({
      queryKey: capabilityRegistryQueryKeys.all,
    })
    void queryClient.invalidateQueries({ queryKey: capabilityQueryKeys.all })
  }
}

/** Subscribes and does the first sync */
export function useAddCapabilityRegistry() {
  const invalidate = useInvalidate()
  return useClientMutation(
    (input: AddCapabilityRegistryInput) =>
      client.addCapabilityRegistry(
        input.url,
        input.kind,
        input.branchPrefix,
        input.userSlug
      ),
    { onSettled: invalidate }
  )
}

export function useRemoveCapabilityRegistry() {
  const invalidate = useInvalidate()
  return useClientMutation(
    (url: string) => client.removeCapabilityRegistry(url),
    { onSettled: invalidate }
  )
}

/** Resolves with the `SyncResult` (whose `error` is set when it failed) */
export function useSyncCapabilityRegistry() {
  const invalidate = useInvalidate()
  return useClientMutation(
    (url: string) => client.syncCapabilityRegistry(url),
    { onSettled: invalidate }
  )
}
