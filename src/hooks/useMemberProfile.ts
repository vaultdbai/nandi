/**
 * A member's profile and per-member trust actions. Every trust change
 * refetches the profiles, the directory, rosters, and the shared lists
 * (a blocked agent or a paused member drops out of them).
 */
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type {
  MemberProfile,
  TrustRelationship,
  TrustUpdate,
} from '@/lib/bindings'
import {
  memberQueryKeys,
  networkQueryKeys,
  rosterQueryKeys,
  unwrapNandi,
  useMemberEvents,
} from './useNetwork'

export function useMemberProfile(instanceId: string) {
  useMemberEvents()
  return useQuery({
    queryKey: memberQueryKeys.byInstance(instanceId),
    queryFn: (): Promise<MemberProfile> =>
      unwrapNandi(client.getMemberByInstance(instanceId)),
    enabled: instanceId !== '',
  })
}

/** The agent ID part of a card ID (`<instance>/<agent>`) */
export function agentIdOf(card: { id: string; instanceId: string }): string {
  const prefix = `${card.instanceId}/`
  return card.id.startsWith(prefix) ? card.id.slice(prefix.length) : card.id
}

/** Writes the new relationship into the cached profile, then refetches */
function useTrustCallbacks() {
  const queryClient = useQueryClient()
  return {
    onSuccess: (trust: TrustRelationship) =>
      queryClient.setQueryData<MemberProfile>(
        memberQueryKeys.byInstance(trust.instanceId),
        profile =>
          profile && {
            ...profile,
            trust,
            agents: profile.agents.map(agent => ({
              ...agent,
              blocked: trust.blockedAgents.includes(agentIdOf(agent.card)),
            })),
          }
      ),
    onSettled: () => {
      for (const queryKey of [
        memberQueryKeys.all,
        networkQueryKeys.all,
        rosterQueryKeys.all,
        ['shared'],
      ]) {
        void queryClient.invalidateQueries({ queryKey })
      }
    },
  }
}

/** Trusted peer and paused delegations; `null` fields keep their value */
export function useSetTrustRelationship() {
  return useMutation({
    mutationFn: (change: { instanceId: string; update: TrustUpdate }) =>
      unwrapNandi(
        client.setTrustRelationship(change.instanceId, change.update)
      ),
    ...useTrustCallbacks(),
  })
}

/** Blocks (or, with `blocked: false`, unblocks) one of a member's agents */
export function useBlockAgentFromMember() {
  return useMutation({
    mutationFn: (change: {
      instanceId: string
      agentId: string
      blocked: boolean
    }) =>
      unwrapNandi(
        change.blocked
          ? client.blockAgentFromMember(change.instanceId, change.agentId)
          : client.unblockAgentFromMember(change.instanceId, change.agentId)
      ),
    ...useTrustCallbacks(),
  })
}

export function usePauseDelegationsFromMember() {
  return useMutation({
    mutationFn: (change: { instanceId: string; paused: boolean }) =>
      unwrapNandi(
        client.pauseDelegationsFromMember(change.instanceId, change.paused)
      ),
    ...useTrustCallbacks(),
  })
}
