/**
 * A team's roster, kept live by `team://roster_changed` (and the other
 * member events, see `useMemberEvents`).
 */
import { useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { TeamRoster } from '@/lib/bindings'
import { rosterQueryKeys, useMemberEvents } from './useNetwork'
import { unwrapResult } from '@/lib/tauri-bindings'

export function useTeamRoster(teamId: string) {
  useMemberEvents()
  return useQuery({
    queryKey: rosterQueryKeys.team(teamId),
    queryFn: (): Promise<TeamRoster> =>
      client.getTeamRoster(teamId).then(unwrapResult),
    enabled: teamId !== '',
  })
}
