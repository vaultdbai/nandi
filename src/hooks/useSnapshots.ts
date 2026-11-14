import { client } from '@/lib/client'
import { useClientQuery } from '@/hooks/useClientQuery'

/**
 * Snapshot queries for data that isn't in the agent store. TanStack Query
 * caches them; `agent-sync` invalidates a run's detail when it finishes.
 */
export function useRunDetail(runId: string) {
  return useClientQuery(
    ['runDetail', runId],
    () => client.getRunDetail(runId),
    { staleTime: 0 }
  )
}

export function useWorkflows() {
  return useClientQuery(['workflows'], () => client.getWorkflows())
}

/** A workflow's definition and recent runs; `workflow://*` events refresh it */
export function useWorkflow(workflowId: string) {
  return useClientQuery(
    ['workflow', workflowId],
    () => client.getWorkflow(workflowId),
    { staleTime: 0 }
  )
}
