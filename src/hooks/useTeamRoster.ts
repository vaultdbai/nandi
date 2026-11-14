/**
 * A team's roster, kept live by `team://roster_changed` (and the other
 * member events, see `useMemberEvents`).
 */
import { client } from '@/lib/client'
import { rosterQueryKeys, useMemberEvents } from './useNetwork'
import { useClientQuery } from '@/hooks/useClientQuery'

export function useTeamRoster(teamId: string) {
  useMemberEvents()
  return useClientQuery(
    rosterQueryKeys.team(teamId),
    () => client.getTeamRoster(teamId),
    { enabled: teamId !== '' }
  )
}
