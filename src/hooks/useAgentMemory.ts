/**
 * An agent's memory (`memory.md` in its folder) and its mutations (wave
 * final, Track C), via TanStack Query. Kept live by `repo://file_changed`:
 * the agent writes new entries in the background after a run, and the file
 * can be edited by hand.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { MemoryEntry } from '@/lib/bindings'

/** Everything memory-related lives under `['agentMemory']` */
export const agentMemoryQueryKeys = {
  all: ['agentMemory'] as const,
  agent: (agentId: string) => ['agentMemory', agentId] as const,
  entries: (agentId: string) => ['agentMemory', agentId, 'entries'] as const,
  enabled: (agentId: string) => ['agentMemory', agentId, 'enabled'] as const,
}

/** Errors as `Error`s, so the UI can show `error.message` */
function unwrap<T>(
  result: { status: 'ok'; data: T } | { status: 'error'; error: string }
): T {
  if (result.status === 'error') throw new Error(result.error)
  return result.data
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
  return useQuery({
    queryKey: agentMemoryQueryKeys.entries(id),
    queryFn: async () => unwrap(await client.loadAgentMemory(id)),
    enabled: Boolean(agentId),
  })
}

/** The agent's memory entries, the disable toggle, and the mutations */
export function useAgentMemory(agentId: string | undefined) {
  const queryClient = useQueryClient()
  const id = agentId ?? ''

  const entries = useAgentMemoryEntries(agentId)
  const enabled = useQuery({
    queryKey: agentMemoryQueryKeys.enabled(id),
    queryFn: async () => unwrap(await client.getAgentMemoryEnabled(id)),
    enabled: Boolean(agentId),
  })

  const invalidate = () =>
    queryClient.invalidateQueries({
      queryKey: agentMemoryQueryKeys.agent(id),
    })

  const update = useMutation({
    mutationFn: async ({
      index,
      content,
    }: {
      index: number
      content: string
    }) => unwrap(await client.updateAgentMemory(id, index, content)),
    onSettled: invalidate,
  })
  const remove = useMutation({
    mutationFn: async (index: number) =>
      unwrap(await client.deleteAgentMemory(id, index)),
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
    onSettled: invalidate,
  })
  const refresh = useMutation({
    mutationFn: async () => unwrap(await client.refreshAgentMemory(id)),
    onSuccess: memory =>
      queryClient.setQueryData(agentMemoryQueryKeys.entries(id), memory),
    onSettled: invalidate,
  })
  const setEnabled = useMutation({
    mutationFn: async (value: boolean) =>
      unwrap(await client.setAgentMemoryEnabled(id, value)),
    onMutate: value => {
      queryClient.setQueryData(agentMemoryQueryKeys.enabled(id), value)
    },
    onSettled: invalidate,
  })

  return { entries, enabled, update, remove, refresh, setEnabled }
}
