/**
 * Trust toward a network member: trusted peer, paused
 * delegations, and blocking — one agent, or the whole member (agent `*`,
 * whose tasks are rejected on arrival).
 */
import type { TrustRelationship } from '@/lib/bindings'
import { useBlockAgentFromMember } from './useMemberProfile'

export {
  usePauseDelegationsFromMember,
  useSetTrustRelationship,
} from './useMemberProfile'

/** The blocked agent ID that stands for every agent of a member */
export const ALL_AGENTS = '*'

/** We blocked the whole member */
export function isMemberBlocked(trust: TrustRelationship | undefined) {
  return trust?.blockedAgents.includes(ALL_AGENTS) ?? false
}

/** Agents of theirs we blocked one by one (not the whole member) */
export function blockedAgentCount(trust: TrustRelationship | undefined) {
  return trust?.blockedAgents.filter(id => id !== ALL_AGENTS).length ?? 0
}

/** Blocks (or unblocks) every agent of a member */
export function useBlockMember() {
  const block = useBlockAgentFromMember()
  return {
    ...block,
    mutate: (
      change: { instanceId: string; blocked: boolean },
      options?: Parameters<typeof block.mutate>[1]
    ) => block.mutate({ ...change, agentId: ALL_AGENTS }, options),
  }
}
