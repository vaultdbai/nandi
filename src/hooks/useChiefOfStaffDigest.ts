/**
 * The Chief of Staff's digest: what it did since the user last looked.
 * Refetches on `request://updated` (any Request change, and after mark-seen).
 */
import { useQueryClient } from '@tanstack/react-query'
import { client, type NandiEventName } from '@/lib/client'
import { useInvalidateOn } from '@/hooks/useInvalidateOn'
import { requestQueryKeys } from '@/hooks/useRequests'
import { useClientQuery, useClientMutation } from '@/hooks/useClientQuery'

export const digestKeys = {
  all: ['chief-of-staff', 'digest'] as const,
}

const DIGEST_EVENTS: readonly NandiEventName[] = ['request://updated']

export function useChiefOfStaffDigest() {
  useInvalidateOn(DIGEST_EVENTS, digestKeys.all)
  return useClientQuery(digestKeys.all, () => client.getChiefOfStaffDigest())
}

export function useApproveQueuedWork() {
  return useClientMutation(
    (queueId: string) => client.approveQueuedWork(queueId),
    {
      invalidates: digestKeys.all,
    }
  )
}

export function useDismissQueuedWork() {
  return useClientMutation(
    (queueId: string) => client.dismissQueuedWork(queueId),
    {
      invalidates: digestKeys.all,
    }
  )
}

/** "Mark done" on a step of a local Request that waits on the user */
export function useCompleteWaitingStep() {
  const queryClient = useQueryClient()
  return useClientMutation(
    (target: { requestId: string; position: number }) =>
      client.completeRequestStep(target.requestId, target.position),
    {
      onSettled: () =>
        Promise.all([
          queryClient.invalidateQueries({ queryKey: digestKeys.all }),
          queryClient.invalidateQueries({ queryKey: requestQueryKeys.all }),
        ]),
    }
  )
}

export function useMarkDigestSeen() {
  const queryClient = useQueryClient()
  return useClientMutation(() => client.markChiefOfStaffDigestSeen(), {
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: digestKeys.all }),
  })
}
