/**
 * One intake item's Chief of Staff thread: its queued items and Requests.
 * Refetches on `request://updated` (the queue or a Request changed), and the
 * conversation is reloaded then too: the backend writes the Chief of
 * Staff's notes into it (queued, approved, dismissed).
 */
import { useEffect } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { loadConversation } from '@/lib/agent-sync'
import { unwrapResult } from '@/lib/tauri-bindings'

export const intakeThreadKeys = {
  detail: (conversationId: string) =>
    ['chief-of-staff', 'thread', conversationId] as const,
}

const THREAD_EVENTS = new Set(['request://updated', 'nandi://resync'])

export function useIntakeThread(conversationId: string) {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (!THREAD_EVENTS.has(event.event)) return
      void queryClient.invalidateQueries({
        queryKey: intakeThreadKeys.detail(conversationId),
      })
      void loadConversation(conversationId)
    })
    return () => subscription.unsubscribe()
  }, [queryClient, conversationId])
  return useQuery({
    queryKey: intakeThreadKeys.detail(conversationId),
    queryFn: async () =>
      unwrapResult(await client.getIntakeThread(conversationId)),
    retry: false,
  })
}
