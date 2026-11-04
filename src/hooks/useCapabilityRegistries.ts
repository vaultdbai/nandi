/**
 * Subscribed capability registries (TanStack Query, refetched on
 * `registry://synced` / `registry://sync_failed`). A sync changes which
 * capabilities exist, so it also invalidates everything under
 * `capabilityQueryKeys.all`.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { RegistryKind } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'
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

  return useQuery({
    queryKey: capabilityRegistryQueryKeys.all,
    queryFn: async () => unwrapResult(await client.listCapabilityRegistries()),
  })
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
  return useMutation({
    mutationFn: async (input: AddCapabilityRegistryInput) =>
      unwrapResult(
        await client.addCapabilityRegistry(
          input.url,
          input.kind,
          input.branchPrefix,
          input.userSlug
        )
      ),
    onSettled: invalidate,
  })
}

export function useRemoveCapabilityRegistry() {
  const invalidate = useInvalidate()
  return useMutation({
    mutationFn: async (url: string) =>
      unwrapResult(await client.removeCapabilityRegistry(url)),
    onSettled: invalidate,
  })
}

/** Resolves with the `SyncResult` (whose `error` is set when it failed) */
export function useSyncCapabilityRegistry() {
  const invalidate = useInvalidate()
  return useMutation({
    mutationFn: async (url: string) =>
      unwrapResult(await client.syncCapabilityRegistry(url)),
    onSettled: invalidate,
  })
}
