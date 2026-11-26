import { client } from '@/lib/client'
import { useClientQuery } from '@/hooks/useClientQuery'

/**
 * When each scheduled trigger runs next. Under `triggers`: a trigger's
 * change refreshes it (`agent-sync`)
 */
export function useNextRuns() {
  return useClientQuery(['triggers', 'nextRuns'], () => client.listNextRuns(), {
    // Times move on: read again every minute while shown
    refetchInterval: 60_000,
  })
}
