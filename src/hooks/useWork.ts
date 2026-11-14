/**
 * Work in flight (TanStack Query): workflow runs and delegated tasks as
 * active / waiting / done items. Refetched on `work://*`, `workflow://*`,
 * and task events, so the view follows runs live.
 */
import { useEffect } from 'react'
import { useQueryClient, type QueryClient } from '@tanstack/react-query'
import { client, type NandiEvent } from '@/lib/client'
import type { WorkState } from '@/lib/bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

export const workKeys = {
  all: ['work'] as const,
  list: (state: WorkState | null) => ['work', 'list', state ?? '*'] as const,
  item: (id: string) => ['work', 'item', id] as const,
  manualSteps: ['work', 'manual-steps'] as const,
}

/** Events after which work items may have changed */
export function changesWork(event: NandiEvent): boolean {
  return (
    event.event.startsWith('work://') ||
    event.event.startsWith('workflow://') ||
    event.event === 'request://updated' ||
    event.event === 'task://state_changed' ||
    event.event === 'task://progress' ||
    event.event === 'nandi://resync'
  )
}

function useWorkEvents(queryClient: QueryClient) {
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (!changesWork(event)) return
      void queryClient.invalidateQueries({ queryKey: workKeys.all })
      // The project view reads the run from its workflow
      if (event.event.startsWith('workflow://')) {
        void queryClient.invalidateQueries({ queryKey: ['workflow'] })
      }
    })
    return subscription.unsubscribe
  }, [queryClient])
}

/** Newest first; `null` for every state */
export function useWork(stateFilter: WorkState | null) {
  const queryClient = useQueryClient()
  useWorkEvents(queryClient)
  return useClientQuery(workKeys.list(stateFilter), () =>
    client.listWork(stateFilter)
  )
}

export function useWorkItem(id: string) {
  const queryClient = useQueryClient()
  useWorkEvents(queryClient)
  return useClientQuery(workKeys.item(id), () => client.getWorkItem(id))
}

/** Manual workflow steps waiting for someone, oldest first */
export function useManualSteps() {
  const queryClient = useQueryClient()
  useWorkEvents(queryClient)
  return useClientQuery(workKeys.manualSteps, () =>
    client.listPendingManualSteps()
  )
}

/** Completes a manual step; its run picks up where it stopped */
export function useCompleteManualStep() {
  return useClientMutation(
    ({
      runId,
      stepId,
      output,
      attachmentIds = [],
    }: {
      runId: string
      stepId: string
      output: string | null
      attachmentIds?: string[]
    }) =>
      client.completeManualStep(runId, stepId, {
        output,
        attachmentIds,
      }),
    { invalidates: workKeys.all }
  )
}

/** Pause, resume, cancel, or rate a work item */
export function useWorkAction() {
  return useClientMutation(
    ({
      id,
      action,
      rating,
    }: {
      id: string
      action: 'pause' | 'resume' | 'cancel' | 'rate'
      rating?: number
    }) => {
      switch (action) {
        case 'pause':
          return client.pauseWork(id)
        case 'resume':
          return client.resumeWork(id)
        case 'cancel':
          return client.cancelWork(id)
        case 'rate':
          return client.rateWork(id, rating ?? 0)
      }
    },
    { invalidates: workKeys.all }
  )
}
