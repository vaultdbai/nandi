import { useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'

export const agentPoliciesQueryKeys = {
  all: ['agent-policies'] as const,
  agent: (agentId: string) => ['agent-policies', agentId] as const,
}

/**
 * The policies in `.nandi/policies/` over an agent: which apply, what each
 * changed from its skill.md, or why it can't run
 */
export function useAgentPolicies(agentId: string) {
  return useQuery({
    queryKey: agentPoliciesQueryKeys.agent(agentId),
    queryFn: async () => unwrapResult(await client.agentPolicies(agentId)),
  })
}
