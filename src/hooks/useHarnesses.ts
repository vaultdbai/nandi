/**
 * Detected harnesses and the default harness (TanStack Query). Refetched
 * on `harness://detected` and `harness://health_changed` (the core
 * re-detects every five minutes, and on Refresh).
 */
import { useEffect } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { HarnessDefault } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

/** Everything harness-related lives under `['harnesses']` */
export const harnessQueryKeys = {
  all: ['harnesses'] as const,
  list: ['harnesses', 'list'] as const,
  default: ['harnesses', 'default'] as const,
  health: (id: string) => ['harnesses', 'health', id] as const,
}

function useHarnessEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (
        event.event === 'harness://detected' ||
        event.event === 'harness://health_changed'
      ) {
        void queryClient.invalidateQueries({ queryKey: harnessQueryKeys.all })
      }
    })
    return subscription.unsubscribe
  }, [queryClient])
}

/** Built-in and custom harnesses with their last detected health */
export function useHarnesses() {
  useHarnessEvents()
  return useClientQuery(harnessQueryKeys.list, () => client.listHarnesses())
}

/** Probes the machine again */
export function useDetectHarnesses() {
  const queryClient = useQueryClient()
  return useClientMutation(() => client.detectHarnesses(), {
    onSuccess: harnesses =>
      queryClient.setQueryData(harnessQueryKeys.list, harnesses),
  })
}

/** Opens and closes a session on the harness */
export function useTestHarness() {
  return useClientMutation((harnessId: string) => client.testHarness(harnessId))
}

/** `harnessId` `null` clears the default */
export function useSetHarnessDefault() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      harnessId,
      model,
    }: {
      harnessId: string | null
      model: string | null
    }): Promise<HarnessDefault | null> => {
      if (harnessId === null) {
        unwrapResult(await client.clearHarnessDefault())
        return null
      }
      return unwrapResult(await client.setHarnessDefault(harnessId, model))
    },
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: harnessQueryKeys.all }),
  })
}
