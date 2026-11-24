/**
 * An agent's playbooks (`skills/<name>.md` next to its agent.md) and their
 * mutations, via TanStack Query. The key sits under the agent definition's
 * (`['agent-definition', id]`), so every file change, live change and
 * Refresh that updates the agent updates its playbooks too.
 */
import { useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { PlaybookInput } from '@/lib/bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

export const playbooksQueryKeys = {
  agent: (agentId: string) =>
    ['agent-definition', agentId, 'playbooks'] as const,
}

export function usePlaybooks(agentId: string) {
  return useClientQuery(playbooksQueryKeys.agent(agentId), () =>
    client.repoAgentPlaybooks(agentId)
  )
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
  return useClientMutation(
    (args: { input: PlaybookInput; previous: string | null }) =>
      client.repoSaveAgentPlaybook(agentId, args.input, args.previous),
    { onSuccess: invalidate }
  )
}

export function useDeletePlaybook(agentId: string) {
  const invalidate = useInvalidatePlaybooks(agentId)
  return useClientMutation(
    (name: string) => client.repoDeleteAgentPlaybook(agentId, name),
    { onSuccess: invalidate }
  )
}
