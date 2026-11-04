/**
 * A team's roster, kept live by `team://roster_changed` (and the other
 * member events, see `useMemberEvents`).
 */
import { useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { TeamRoster } from '@/lib/bindings'
import { rosterQueryKeys, unwrapNandi, useMemberEvents } from './useNetwork'

export function useTeamRoster(teamId: string) {
  useMemberEvents()
  return useQuery({
    queryKey: rosterQueryKeys.team(teamId),
    queryFn: (): Promise<TeamRoster> =>
      unwrapNandi(client.getTeamRoster(teamId)),
    enabled: teamId !== '',
  })
}
