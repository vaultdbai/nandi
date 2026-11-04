/**
 * Harness sessions and their recorded ACP events (TanStack Query).
 * Refetched when a session starts or ends; a live session's events also
 * on every `harness://turn_delta`.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { HarnessEventRecord, HarnessSessionSummary } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

/** Everything session-related lives under `['harness-sessions']` */
export const harnessSessionKeys = {
  all: ['harness-sessions'] as const,
  list: (agentId: string | null) =>
    ['harness-sessions', 'list', agentId ?? '*'] as const,
  events: (sessionId: string) =>
    ['harness-sessions', 'events', sessionId] as const,
}

function useSessionEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (
        event.event === 'harness://session_started' ||
        event.event === 'harness://session_ended' ||
        event.event === 'nandi://resync'
      ) {
        void queryClient.invalidateQueries({
          queryKey: harnessSessionKeys.all,
        })
      }
    })
    return subscription.unsubscribe
  }, [queryClient])
}

/** Newest first; `agentId` `null` for every agent's */
export function useHarnessSessions(agentId: string | null) {
  useSessionEvents()
  return useQuery({
    queryKey: harnessSessionKeys.list(agentId),
    queryFn: async (): Promise<HarnessSessionSummary[]> =>
      unwrapResult(await client.listHarnessSessions(agentId, null)),
  })
}

/** A session's recorded updates (idle while `null`) */
export function useHarnessEvents(sessionId: string | null) {
  const queryClient = useQueryClient()
  useEffect(() => {
    if (!sessionId) return
    const subscription = client.subscribe(event => {
      if (event.event === 'harness://turn_delta') {
        const payload = event.payload as { sessionId?: string } | null
        if (payload?.sessionId === sessionId) {
          void queryClient.invalidateQueries({
            queryKey: harnessSessionKeys.events(sessionId),
          })
        }
      }
    })
    return subscription.unsubscribe
  }, [queryClient, sessionId])
  return useQuery({
    queryKey: harnessSessionKeys.events(sessionId ?? ''),
    queryFn: async (): Promise<HarnessEventRecord[]> =>
      unwrapResult(await client.getHarnessEvents(sessionId ?? '', null)),
    enabled: !!sessionId,
  })
}

/** Stops a live session's CLI (and container); its run fails */
export function useKillHarnessSession() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (sessionId: string) =>
      unwrapResult(await client.killHarnessSession(sessionId)),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: harnessSessionKeys.all }),
  })
}
