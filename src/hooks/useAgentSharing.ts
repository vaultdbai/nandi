/**
 * How one of my agents is shared, and the toggle that changes it: whether
 * relay peers may call it (`accepts_delegation`). Used by the sidebar agent
 * menu and the sharing dialog. Nothing here shares outside the relay.
 */
import { useMutation, useQueryClient } from '@tanstack/react-query'
import type { AgentDelegationSettings, SharedAgent } from '@/lib/bindings'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'
import { delegationQueryKeys } from './useDelegation'
import { sharedQueryKeys, useSharedByMe } from './useSharedAgents'

export interface AgentSharingState {
  /** Relay peers may send it tasks */
  relay: boolean
  /** Its entry in "shared by me", when it's shared at all */
  shared: SharedAgent | undefined
}

/** How `agentId` is shared, from the "shared by me" list */
export function sharingOf(
  byMe: SharedAgent[] | undefined,
  agentId: string
): AgentSharingState {
  const shared = byMe?.find(agent => agentIdOfCard(agent) === agentId)
  return {
    relay: shared?.sources.includes('relay') ?? false,
    shared,
  }
}

function agentIdOfCard(agent: SharedAgent): string {
  const { id, instanceId } = agent.card
  return id.startsWith(`${instanceId}/`) ? id.slice(instanceId.length + 1) : id
}

/** How `agentId` is shared with the relay */
export function useAgentSharing(agentId: string) {
  const byMe = useSharedByMe()
  return {
    sharing: sharingOf(byMe.data, agentId),
    loading: byMe.isLoading,
  }
}

/** Every query that shows how agents are shared */
function invalidateSharing(
  queryClient: ReturnType<typeof useQueryClient>,
  agentId: string
) {
  void queryClient.invalidateQueries({ queryKey: sharedQueryKeys.all })
  void queryClient.invalidateQueries({
    queryKey: delegationQueryKeys.settings(agentId),
  })
  void queryClient.invalidateQueries({
    queryKey: delegationQueryKeys.providersAll,
  })
}

/**
 * Turns relay delegation on or off, keeping the agent's other delegation
 * settings (outbound, limit, overrides) as they are.
 */
export function useSetRelaySharing(agentId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (on: boolean) => {
      const current: AgentDelegationSettings = unwrapResult(
        await client.getAgentDelegation(agentId)
      )
      unwrapResult(
        await client.setAgentDelegation(agentId, {
          ...current,
          acceptInbound: on,
        })
      )
    },
    onSettled: () => invalidateSharing(queryClient, agentId),
  })
}
