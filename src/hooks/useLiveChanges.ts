/**
 * Live UI hooks: the recent file changes (who changed what) and the
 * Refresh action. `live://changed` keeps the list current
 * (`lib/live-changes.ts`), so it never polls.
 */
import { useMutation, useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { ChangeArea, LiveChange } from '@/lib/bindings'
import { liveQueryKeys, refreshFromDisk } from '@/lib/live-changes'
import { unwrapResult } from '@/lib/tauri-bindings'

/** Newest first */
export function useRecentChanges() {
  return useQuery({
    queryKey: liveQueryKeys.recent,
    queryFn: async () => unwrapResult(await client.listRecentChanges()),
    staleTime: Infinity,
  })
}

/**
 * The latest change to one thing, if the app saw one: a note, or an agent
 * (`['agent', 'work']`: its definition or its work folder)
 */
export function useLatestChange(
  areas: readonly ChangeArea[],
  targetId: string | null
): LiveChange | null {
  const { data } = useRecentChanges()
  if (!targetId) return null
  return (
    data?.find(
      change => areas.includes(change.area) && change.targetId === targetId
    ) ?? null
  )
}

/** The Refresh button: re-reads everything from disk */
export function useRefreshFromDisk() {
  return useMutation({ mutationFn: refreshFromDisk })
}
