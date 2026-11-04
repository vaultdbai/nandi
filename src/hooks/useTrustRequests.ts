/**
 * People refused because this Nandi doesn't trust them yet (task 12), for
 * Network › Sharing and Home's Inbox. Kept live by
 * `trust://request_received` (someone was refused, or answered) and
 * `trust://relationship_updated` (trusting them elsewhere clears it).
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { TrustAnswer, TrustRequest } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

export const trustRequestKeys = {
  all: ['trust', 'requests'] as const,
}

const EVENTS = new Set([
  'trust://request_received',
  'trust://relationship_updated',
  'nandi://resync',
])

export function useTrustRequests() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (EVENTS.has(event.event)) {
        void queryClient.invalidateQueries({ queryKey: trustRequestKeys.all })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return useQuery({
    queryKey: trustRequestKeys.all,
    queryFn: async (): Promise<TrustRequest[]> =>
      unwrapResult(await client.listTrustRequests()),
    // Unavailable (e.g. no relay yet): nothing is waiting
    retry: false,
  })
}

export function useRespondTrustRequest() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (input: {
      instanceId: string
      answer: TrustAnswer
      block?: boolean
    }) =>
      unwrapResult(
        await client.respondTrustRequest(
          input.instanceId,
          input.answer,
          input.block ?? false
        )
      ),
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: trustRequestKeys.all })
      // Trusted or blocked: profiles, the directory and callers change too
      for (const queryKey of [['members'], ['network'], ['gateway']]) {
        void queryClient.invalidateQueries({ queryKey })
      }
    },
  })
}

/** "Sam K. (@sam)", or the handle alone, or a plain fallback, never an ID */
export function requesterName(
  request: Pick<TrustRequest, 'memberName' | 'handle'>,
  unnamed: string
): string {
  if (request.memberName && request.handle) {
    return `${request.memberName} (@${request.handle})`
  }
  if (request.memberName) return request.memberName
  if (request.handle) return `@${request.handle}`
  return unnamed
}
