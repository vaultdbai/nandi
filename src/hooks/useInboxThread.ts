/**
 * One Inbox item's Ask chat with the Chief of Staff (`open_inbox_thread`):
 * made the first time it's opened, with whether the item still waits and
 * which decisions the chat can propose. Refetched when the item may have
 * been decided (an approval, a hold, an assignment, a manual step or a
 * Request changed).
 */
import { useEffect } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'

export const inboxThreadKeys = {
  detail: (conversationId: string) =>
    ['inbox', 'thread', conversationId] as const,
}

const THREAD_EVENTS = new Set([
  'agent://approval_resolved',
  'gateway://hold_changed',
  'gateway://assignment_changed',
  'workflow://manual_step_completed',
  'request://updated',
  'request://returned',
  'nandi://resync',
])

export function useInboxThread(conversationId: string) {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (!THREAD_EVENTS.has(event.event)) return
      void queryClient.invalidateQueries({
        queryKey: inboxThreadKeys.detail(conversationId),
      })
    })
    return () => subscription.unsubscribe()
  }, [queryClient, conversationId])
  return useQuery({
    queryKey: inboxThreadKeys.detail(conversationId),
    queryFn: async () =>
      unwrapResult(await client.openInboxThread(conversationId)),
    retry: false,
  })
}
