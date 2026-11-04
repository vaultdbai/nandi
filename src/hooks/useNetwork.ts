/**
 * The network directory, kept live by `network://*`. Also home of the
 * event wiring every member-facing query shares: `network://*`,
 * `member://*`, `trust://*`, and `team://roster_changed` refetch the
 * directory, member profiles, rosters, and (on trust changes) the shared
 * lists.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { NetworkMember, NetworkSnapshot, Result } from '@/lib/bindings'

export const networkQueryKeys = {
  all: ['network'] as const,
  snapshot: ['network', 'snapshot'] as const,
}

export const memberQueryKeys = {
  all: ['members'] as const,
  byInstance: (instanceId: string) =>
    ['members', 'instance', instanceId] as const,
}

export const rosterQueryKeys = {
  all: ['roster'] as const,
  team: (teamId: string) => ['roster', teamId] as const,
}

/** Rejects with an `Error` carrying the core's message */
export async function unwrapNandi<T>(
  result: Promise<Result<T, string>>
): Promise<T> {
  const resolved = await result
  if (resolved.status === 'error') throw new Error(resolved.error)
  return resolved.data
}

const MEMBER_EVENT_PREFIXES = ['network://', 'member://', 'trust://']

/** Whether a core event should refetch the member-facing queries */
export function isMemberEvent(name: string): boolean {
  return (
    name === 'team://roster_changed' ||
    MEMBER_EVENT_PREFIXES.some(prefix => name.startsWith(prefix))
  )
}

/** Refetches the directory, profiles, and rosters on member events */
export function useMemberEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (!isMemberEvent(event.event)) return
      for (const queryKey of [
        networkQueryKeys.all,
        memberQueryKeys.all,
        rosterQueryKeys.all,
      ]) {
        void queryClient.invalidateQueries({ queryKey })
      }
      if (event.event.startsWith('trust://')) {
        void queryClient.invalidateQueries({ queryKey: ['shared'] })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])
}

/** The directory; `enabled: false` reads the cache without fetching */
export function useNetworkSnapshot(enabled = true) {
  useMemberEvents()
  return useQuery({
    queryKey: networkQueryKeys.snapshot,
    queryFn: (): Promise<NetworkSnapshot> =>
      unwrapNandi(client.getNetworkSnapshot()),
    enabled,
  })
}

/**
 * A member's display name from the cached directory (never fetches); the
 * instance ID when the directory doesn't know them, `null` for `null`
 */
export function useMemberName(instanceId: string | null): string | null {
  const snapshot = useQuery({
    queryKey: networkQueryKeys.snapshot,
    queryFn: (): Promise<NetworkSnapshot> =>
      unwrapNandi(client.getNetworkSnapshot()),
    enabled: false,
  })
  if (instanceId === null) return null
  const member = snapshot.data?.members.find(m => m.instanceId === instanceId)
  return member?.displayName || instanceId
}

/** Asks the relay again, bypassing the 10s cache */
export function useRefreshNetwork() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (): Promise<NetworkSnapshot> =>
      unwrapNandi(client.refreshNetwork()),
    onSuccess: snapshot =>
      queryClient.setQueryData(networkQueryKeys.snapshot, snapshot),
  })
}

export type MemberFilter = 'all' | 'online' | 'recent'

/** "Recently active": seen within the last day */
export const RECENTLY_ACTIVE_MS = 24 * 60 * 60 * 1000

/** Members matching `filter` (order kept: online first, then by name) */
export function filterMembers(
  members: NetworkMember[],
  filter: MemberFilter,
  now: number = Date.now()
): NetworkMember[] {
  switch (filter) {
    case 'all':
      return members
    case 'online':
      return members.filter(member => member.online)
    case 'recent':
      return members.filter(
        member =>
          member.online ||
          (member.lastSeen !== null &&
            now - member.lastSeen <= RECENTLY_ACTIVE_MS)
      )
  }
}

/** Up to two initials for an avatar fallback */
export function initialsOf(name: string): string {
  const words = name
    .trim()
    .split(/[\s._-]+/)
    .filter(Boolean)
  const letters =
    words.length > 1
      ? [words[0]?.[0], words[1]?.[0]]
      : [words[0]?.[0], words[0]?.[1]]
  return letters.filter(Boolean).join('').toUpperCase() || '?'
}

/**
 * Members whose name, handle, description, capabilities or
 * responsibilities match `query`
 */
export function searchMembers(
  members: NetworkMember[],
  query: string
): NetworkMember[] {
  const q = query.trim().toLowerCase()
  if (!q) return members
  return members.filter(member =>
    [
      member.displayName,
      member.handle ?? '',
      member.gateway.description,
      ...member.gateway.capabilities,
      ...(member.gateway.responsibilities ?? []),
    ].some(text => text.toLowerCase().includes(q))
  )
}
