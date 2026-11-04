import { useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'

/**
 * Snapshot queries for data that isn't in the agent store. TanStack Query
 * caches them; `agent-sync` invalidates a run's detail when it finishes.
 */
export function useRunDetail(runId: string) {
  return useQuery({
    queryKey: ['runDetail', runId],
    queryFn: async () => unwrapResult(await client.getRunDetail(runId)),
    staleTime: 0,
  })
}

export function useWorkflows() {
  return useQuery({
    queryKey: ['workflows'],
    queryFn: async () => unwrapResult(await client.getWorkflows()),
  })
}

/** A workflow's definition and recent runs; `workflow://*` events refresh it */
export function useWorkflow(workflowId: string) {
  return useQuery({
    queryKey: ['workflow', workflowId],
    queryFn: async () => unwrapResult(await client.getWorkflow(workflowId)),
    staleTime: 0,
  })
}
