/**
 * Harness sessions and their recorded ACP events (TanStack Query).
 * Refetched when a session starts or ends; a live session's events also
 * on every `harness://turn_delta`.
 */
import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

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
        event.event === 'harness://session_ended'
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
  return useClientQuery(harnessSessionKeys.list(agentId), () =>
    client.listHarnessSessions(agentId, null)
  )
}

/** Stops a live session's CLI (and container); its run fails */
export function useKillHarnessSession() {
  return useClientMutation(
    (sessionId: string) => client.killHarnessSession(sessionId),
    { invalidates: harnessSessionKeys.all }
  )
}
