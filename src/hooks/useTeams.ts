/**
 * Teams and their registries, kept live by `team://changed`.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type {
  AgentCard,
  Result,
  ShareResult,
  TaskResult,
  Team,
  TeamDetail,
} from '@/lib/bindings'

export const teamsQueryKeys = {
  all: ['teams'] as const,
  list: ['teams', 'list'] as const,
  detail: (teamId: string) => ['teams', 'detail', teamId] as const,
}

async function unwrap<T>(result: Promise<Result<T, string>>): Promise<T> {
  const resolved = await result
  if (resolved.status === 'error') throw new Error(resolved.error)
  return resolved.data
}

/** Refetches the team queries whenever the core says a team changed */
function useTeamEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (event.event === 'team://changed') {
        void queryClient.invalidateQueries({ queryKey: teamsQueryKeys.list })
        void queryClient.invalidateQueries({
          queryKey: teamsQueryKeys.detail(event.payload.teamId),
        })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])
}

/** The teams I'm in */
export function useTeams() {
  useTeamEvents()
  return useQuery({
    queryKey: teamsQueryKeys.list,
    queryFn: (): Promise<Team[]> => unwrap(client.listTeams()),
  })
}

/** One team with its registry's agents and packs */
export function useTeam(teamId: string | null) {
  return useQuery({
    queryKey: teamsQueryKeys.detail(teamId ?? ''),
    queryFn: (): Promise<TeamDetail> => unwrap(client.getTeam(teamId ?? '')),
    enabled: teamId !== null,
  })
}

export type TeamAction = 'create' | 'join' | 'leave' | 'sync'

/** Create (`teamId` is the registry URL), join, leave, or sync a team */
export function useTeamAction() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      teamId,
      action,
    }: {
      teamId: string
      action: TeamAction
    }): Promise<void> => {
      switch (action) {
        case 'create':
          await unwrap(client.createTeam(teamId))
          return
        case 'join':
          await unwrap(client.joinTeam(teamId))
          return
        case 'leave':
          await unwrap(client.leaveTeam(teamId))
          return
        case 'sync':
          await unwrap(client.syncTeamRegistry(teamId))
          return
      }
    },
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: teamsQueryKeys.all }),
  })
}

/** Shares a local agent to a team, or stops sharing it */
export function useShareAgent() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      teamId,
      agentId,
      share,
    }: {
      teamId: string
      agentId: string
      share: boolean
    }): Promise<ShareResult | null> => {
      if (!share) {
        await unwrap(client.unshareAgent(teamId, agentId))
        return null
      }
      // `branch`: pushed for a team admin to merge, not yet in the index
      return unwrap(client.shareAgent(teamId, agentId))
    },
    onSettled: (_data, _error, { teamId }) =>
      queryClient.invalidateQueries({
        queryKey: teamsQueryKeys.detail(teamId),
      }),
  })
}

/** How many entries the feed shows */
export const TEAM_ACTIVITY_LIMIT = 10

/**
 * Delegations that involve the team: to or from one of its shared agents,
 * or with one of its members. Newest first.
 */
export function teamActivity(
  history: TaskResult[],
  team: Team,
  cards: AgentCard[],
  limit = TEAM_ACTIVITY_LIMIT
): TaskResult[] {
  const cardIds = new Set(cards.map(card => card.id))
  const handles = new Set(
    team.members.map(member => member.githubHandle.toLowerCase())
  )
  const isMember = (owner: string | null) =>
    owner !== null && handles.has(owner.toLowerCase())
  return history
    .filter(
      entry =>
        cardIds.has(entry.providerAgent) ||
        cardIds.has(entry.requesterAgent) ||
        isMember(entry.providerOwner) ||
        isMember(entry.requesterOwner)
    )
    .sort((a, b) => b.startedAt - a.startedAt)
    .slice(0, limit)
}
