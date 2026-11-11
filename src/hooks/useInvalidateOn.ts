import { useEffect, useRef } from 'react'
import { useQueryClient, type QueryKey } from '@tanstack/react-query'
import { client, type NandiEventName } from '@/lib/client'

/**
 * Refetches `queryKey` whenever the core emits one of `eventNames`, and
 * after a `nandi://resync` (events may have been missed), which every list
 * gets without naming it. The main window's agent-sync also invalidates
 * everything on a resync; this one is for the companion window, which runs
 * these hooks (home feed, requests) with no agent-sync.
 */
export function useInvalidateOn(
  eventNames: readonly NandiEventName[],
  queryKey: QueryKey
) {
  const queryClient = useQueryClient()
  // The latest key, so a key built on each render doesn't resubscribe
  const latestKey = useRef(queryKey)
  useEffect(() => {
    latestKey.current = queryKey
  })

  useEffect(() => {
    const names = new Set<NandiEventName>([...eventNames, 'nandi://resync'])
    const subscription = client.subscribe(event => {
      if (names.has(event.event)) {
        void queryClient.invalidateQueries({ queryKey: latestKey.current })
      }
    })
    return () => subscription.unsubscribe()
  }, [eventNames, queryClient])
}
