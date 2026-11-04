import { useEffect } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { localMidnight } from '@/lib/agent-sync'
import { logger } from '@/lib/logger'
import { client } from '@/lib/client'
import type { AgentInfo } from '@/lib/tauri-bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

export const agentsQueryKeys = {
  all: ['agents'] as const,
  /** Under `all`: agent changes (saves, deletes) refresh it too */
  overviews: (todayStart: number) =>
    ['agents', 'overviews', todayStart] as const,
}

/**
 * Each agent's runtime, model and sharing plus its last seven days, for
 * the Agents list and the agent page's health card. A finished turn
 * refreshes it.
 */
export function useAgentOverviews() {
  const queryClient = useQueryClient()
  const todayStart = localMidnight()

  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (
        event.event === 'agent://turn_done' ||
        event.event === 'nandi://resync'
      ) {
        void queryClient.invalidateQueries({
          queryKey: ['agents', 'overviews'],
        })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return useQuery({
    queryKey: agentsQueryKeys.overviews(todayStart),
    queryFn: async () =>
      unwrapResult(await client.listAgentOverviews(todayStart)),
  })
}

/**
 * Agents discovered in `~/.nandi/agents`, with live status from Docker.
 */
export function useAgents() {
  return useQuery({
    queryKey: agentsQueryKeys.all,
    queryFn: async (): Promise<AgentInfo[]> => {
      const result = await client.listAgents()

      if (result.status === 'error') {
        logger.error('Failed to list agents', { error: result.error })
        throw new Error(result.error)
      }

      return result.data
    },
    staleTime: 1000 * 5, // 5 seconds
  })
}
