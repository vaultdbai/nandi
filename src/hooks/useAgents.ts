import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { localMidnight } from '@/lib/agent-sync'
import { logger } from '@/lib/logger'
import { client } from '@/lib/client'
import { useClientQuery } from '@/hooks/useClientQuery'

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

  return useClientQuery(agentsQueryKeys.overviews(todayStart), () =>
    client.listAgentOverviews(todayStart)
  )
}

/**
 * Agents discovered in `~/.nandi/agents`, with live status from Docker.
 */
export function useAgents() {
  return useClientQuery(
    agentsQueryKeys.all,
    async () => {
      const result = await client.listAgents()

      if (result.status === 'error') {
        logger.error('Failed to list agents', { error: result.error })
      }

      return result
    },
    {
      staleTime: 1000 * 5, // 5 seconds
    }
  )
}
