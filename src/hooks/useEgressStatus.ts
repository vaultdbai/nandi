import { useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'

export const egressStatusQueryKey = ['egressStatus'] as const

/** New egress.log entries aren't pushed as events, so poll while visible */
const REFRESH_MS = 5_000

/** AWS credential status (never the secret), proxy routes, recent egress log. */
export function useEgressStatus() {
  return useQuery({
    queryKey: egressStatusQueryKey,
    queryFn: async () => unwrapResult(await client.getEgressStatus()),
    refetchInterval: REFRESH_MS,
  })
}
