/**
 * One Inbox item's Ask chat with the Chief of Staff (`open_inbox_thread`):
 * made the first time it's opened, with whether the item still waits and
 * which decisions the chat can propose. Refetched when the item may have
 * been decided (an approval, a hold, an assignment, a manual step or a
 * Request changed).
 */
import { useQuery } from '@tanstack/react-query'
import { client, type NandiEventName } from '@/lib/client'
import { useInvalidateOn } from '@/hooks/useInvalidateOn'
import { unwrapResult } from '@/lib/tauri-bindings'

export const inboxThreadKeys = {
  detail: (conversationId: string) =>
    ['inbox', 'thread', conversationId] as const,
}

const THREAD_EVENTS: readonly NandiEventName[] = [
  'agent://approval_resolved',
  'gateway://hold_changed',
  'gateway://assignment_changed',
  'workflow://manual_step_completed',
  'request://updated',
  'request://returned',
]

export function useInboxThread(conversationId: string) {
  useInvalidateOn(THREAD_EVENTS, inboxThreadKeys.detail(conversationId))
  return useQuery({
    queryKey: inboxThreadKeys.detail(conversationId),
    queryFn: async () =>
      unwrapResult(await client.openInboxThread(conversationId)),
    retry: false,
  })
}
