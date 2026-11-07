/**
 * The Chief of Staff's digest: what it did since the user last looked.
 * Refetches on `request://updated` (any Request change, and after mark-seen).
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'
import { requestQueryKeys } from '@/hooks/useRequests'

export const digestKeys = {
  all: ['chief-of-staff', 'digest'] as const,
}

const DIGEST_EVENTS = new Set(['request://updated', 'nandi://resync'])

export function useChiefOfStaffDigest() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (DIGEST_EVENTS.has(event.event)) {
        void queryClient.invalidateQueries({ queryKey: digestKeys.all })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])
  return useQuery({
    queryKey: digestKeys.all,
    queryFn: async () => unwrapResult(await client.getChiefOfStaffDigest()),
  })
}

export function useApproveQueuedWork() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (queueId: string) =>
      unwrapResult(await client.approveQueuedWork(queueId)),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: digestKeys.all }),
  })
}

export function useDismissQueuedWork() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (queueId: string) =>
      unwrapResult(await client.dismissQueuedWork(queueId)),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: digestKeys.all }),
  })
}

/** "Mark done" on a step of a local Request that waits on the user */
export function useCompleteWaitingStep() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (target: { requestId: string; position: number }) =>
      unwrapResult(
        await client.completeRequestStep(target.requestId, target.position)
      ),
    onSettled: () =>
      Promise.all([
        queryClient.invalidateQueries({ queryKey: digestKeys.all }),
        queryClient.invalidateQueries({ queryKey: requestQueryKeys.all }),
      ]),
  })
}

export function useMarkDigestSeen() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async () =>
      unwrapResult(await client.markChiefOfStaffDigestSeen()),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: digestKeys.all }),
  })
}
