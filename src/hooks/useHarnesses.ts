/**
 * Detected harnesses and the default harness (TanStack Query). Refetched
 * on `harness://detected` and `harness://health_changed` (the core
 * re-detects every five minutes, and on Refresh).
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type {
  HarnessDefault,
  HarnessHealth,
  HarnessInfo,
  HarnessTestResult,
} from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

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
        event.event === 'harness://health_changed' ||
        event.event === 'nandi://resync'
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
  return useQuery({
    queryKey: harnessQueryKeys.list,
    queryFn: async (): Promise<HarnessInfo[]> =>
      unwrapResult(await client.listHarnesses()),
  })
}

export function useHarnessHealth(harnessId: string | null) {
  return useQuery({
    queryKey: harnessQueryKeys.health(harnessId ?? ''),
    queryFn: async (): Promise<HarnessHealth> =>
      unwrapResult(await client.getHarnessHealth(harnessId ?? '')),
    enabled: !!harnessId,
  })
}

/** The harness new agents get (`null`: none chosen) */
export function useHarnessDefault() {
  return useQuery({
    queryKey: harnessQueryKeys.default,
    queryFn: async (): Promise<HarnessDefault | null> =>
      unwrapResult(await client.getHarnessDefault()),
  })
}

/** Probes the machine again */
export function useDetectHarnesses() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (): Promise<HarnessInfo[]> =>
      unwrapResult(await client.detectHarnesses()),
    onSuccess: harnesses =>
      queryClient.setQueryData(harnessQueryKeys.list, harnesses),
  })
}

/** Opens and closes a session on the harness */
export function useTestHarness() {
  return useMutation({
    mutationFn: async (harnessId: string): Promise<HarnessTestResult> =>
      unwrapResult(await client.testHarness(harnessId)),
  })
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
