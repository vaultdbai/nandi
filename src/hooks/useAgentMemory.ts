/**
 * An agent's memory (`memory.md` in its folder) and its mutations, via
 * TanStack Query. Kept live by `repo://file_changed`:
 * the agent writes new entries in the background after a run, and the file
 * can be edited by hand.
 */
import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { MemoryEntry } from '@/lib/bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

/** Everything memory-related lives under `['agentMemory']` */
export const agentMemoryQueryKeys = {
  all: ['agentMemory'] as const,
  agent: (agentId: string) => ['agentMemory', agentId] as const,
  entries: (agentId: string) => ['agentMemory', agentId, 'entries'] as const,
  enabled: (agentId: string) => ['agentMemory', agentId, 'enabled'] as const,
}

/** Refetches the agent's memory when a file in its folder changes */
function useMemoryFileEvents(agentId: string | undefined) {
  const queryClient = useQueryClient()
  useEffect(() => {
    if (!agentId) return
    const subscription = client.subscribe(event => {
      if (
        event.event === 'repo://file_changed' &&
        event.payload.kind === 'agent' &&
        event.payload.id === agentId
      ) {
        void queryClient.invalidateQueries({
          queryKey: agentMemoryQueryKeys.agent(agentId),
        })
      }
    })
    return () => subscription.unsubscribe()
  }, [agentId, queryClient])
}

/** The agent's memory entries (file order: oldest first), kept live */
export function useAgentMemoryEntries(agentId: string | undefined) {
  const id = agentId ?? ''
  useMemoryFileEvents(agentId)
  return useClientQuery(
    agentMemoryQueryKeys.entries(id),
    () => client.loadAgentMemory(id),
    { enabled: Boolean(agentId) }
  )
}

/** The agent's memory entries, the disable toggle, and the mutations */
export function useAgentMemory(agentId: string | undefined) {
  const queryClient = useQueryClient()
  const id = agentId ?? ''

  const entries = useAgentMemoryEntries(agentId)
  const enabled = useClientQuery(
    agentMemoryQueryKeys.enabled(id),
    () => client.getAgentMemoryEnabled(id),
    { enabled: Boolean(agentId) }
  )

  const invalidates = agentMemoryQueryKeys.agent(id)

  const update = useClientMutation(
    ({ index, content }: { index: number; content: string }) =>
      client.updateAgentMemory(id, index, content),
    { invalidates }
  )
  const remove = useClientMutation(
    (index: number) => client.deleteAgentMemory(id, index),
    {
      invalidates,
      // Gone from the list at once; the refetch confirms
      onMutate: async index => {
        const key = agentMemoryQueryKeys.entries(id)
        await queryClient.cancelQueries({ queryKey: key })
        const previous = queryClient.getQueryData<MemoryEntry[]>(key)
        if (previous) {
          queryClient.setQueryData(
            key,
            previous.filter((_, i) => i !== index)
          )
        }
        return { previous }
      },
      onError: (_error, _index, context) => {
        if (context?.previous) {
          queryClient.setQueryData(
            agentMemoryQueryKeys.entries(id),
            context.previous
          )
        }
      },
    }
  )
  const refresh = useClientMutation(() => client.refreshAgentMemory(id), {
    invalidates,
    onSuccess: memory =>
      queryClient.setQueryData(agentMemoryQueryKeys.entries(id), memory),
  })
  const setEnabled = useClientMutation(
    (value: boolean) => client.setAgentMemoryEnabled(id, value),
    {
      invalidates,
      onMutate: value => {
        queryClient.setQueryData(agentMemoryQueryKeys.enabled(id), value)
      },
    }
  )

  return { entries, enabled, update, remove, refresh, setEnabled }
}
