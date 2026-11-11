/**
 * Requests (v1, phase 1): one unit of work with its Steps, projected by
 * the core from the task tables. Kept live by the events that change a
 * task, its child run, or its work item.
 */
import { useQueryClient } from '@tanstack/react-query'
import { client, type NandiEventName } from '@/lib/client'
import { useInvalidateOn } from '@/hooks/useInvalidateOn'
import { useClientQuery, useClientMutation } from '@/hooks/useClientQuery'

export const requestQueryKeys = {
  all: ['requests'] as const,
  detail: (id: string) => ['requests', 'detail', id] as const,
  forRun: (runId: string) => ['requests', 'for-run', runId] as const,
}

/** Events after which a Request may look different */
export const REQUEST_EVENTS: readonly NandiEventName[] = [
  'task://state_changed',
  'task://progress',
  'task://timed_out',
  'task://reassigned',
  'task://circuit_opened',
  'task://clarification_requested',
  'task://clarification_answered',
  'work://item_updated',
  'work://item_added',
  'agent://run',
  'agent://turn_done',
  'agent://approval_request',
  'agent://approval_resolved',
  'request://updated',
]

export function useRequest(id: string) {
  useInvalidateOn(REQUEST_EVENTS, requestQueryKeys.all)

  return useClientQuery(
    requestQueryKeys.detail(id),
    () => client.getRequest(id),
    {
      retry: false,
    }
  )
}

/** The Request a run belongs to (`null`: none), for Run Detail's breadcrumb */
export function useRequestForRun(runId: string) {
  return useClientQuery(
    requestQueryKeys.forRun(runId),
    () => client.requestForRun(runId),
    {
      retry: false,
      staleTime: 60_000,
    }
  )
}

/** Marks a step of a local Request that waits on the user done */
export function useCompleteRequestStep(id: string) {
  return useClientMutation(
    (position: number) => client.completeRequestStep(id, position),
    {
      invalidates: requestQueryKeys.all,
    }
  )
}

/** Do this again: the same plan as a new Request (resolves to its ID) */
export function useRerunRequest() {
  return useClientMutation((id: string) => client.rerunRequest(id))
}

/** Tries a failed outgoing Request again on the next provider */
export function useRetryRequest() {
  const queryClient = useQueryClient()
  return useClientMutation((id: string) => client.retryTask(id), {
    onSuccess: request =>
      queryClient.setQueryData(requestQueryKeys.detail(request.id), request),
  })
}
