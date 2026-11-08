/**
 * An agent's playbooks (`skills/<name>.md` next to its skill.md) and their
 * mutations, via TanStack Query. The key sits under the agent definition's
 * (`['agent-definition', id]`), so every file change, live change and
 * Refresh that updates the agent updates its playbooks too.
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { PlaybookInput } from '@/lib/bindings'

export const playbooksQueryKeys = {
  agent: (agentId: string) =>
    ['agent-definition', agentId, 'playbooks'] as const,
}

/** Errors as `Error`s, so the UI can show `error.message` */
function unwrap<T>(
  result: { status: 'ok'; data: T } | { status: 'error'; error: string }
): T {
  if (result.status === 'error') throw new Error(result.error)
  return result.data
}

export function usePlaybooks(agentId: string) {
  return useQuery({
    queryKey: playbooksQueryKeys.agent(agentId),
    queryFn: async () => unwrap(await client.agentPlaybooks(agentId)),
  })
}

function useInvalidatePlaybooks(agentId: string) {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({
      queryKey: playbooksQueryKeys.agent(agentId),
    })
    void queryClient.invalidateQueries({ queryKey: ['git-status'] })
  }
}

/** Saves one playbook; `previous` is its name before the edit (`null`: new) */
export function useSavePlaybook(agentId: string) {
  const invalidate = useInvalidatePlaybooks(agentId)
  return useMutation({
    mutationFn: async (args: {
      input: PlaybookInput
      previous: string | null
    }) =>
      unwrap(
        await client.saveAgentPlaybook(agentId, args.input, args.previous)
      ),
    onSuccess: invalidate,
  })
}

export function useDeletePlaybook(agentId: string) {
  const invalidate = useInvalidatePlaybooks(agentId)
  return useMutation({
    mutationFn: async (name: string) =>
      unwrap(await client.deleteAgentPlaybook(agentId, name)),
    onSuccess: invalidate,
  })
}
