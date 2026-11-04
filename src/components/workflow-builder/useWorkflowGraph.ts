import { useEffect } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { WorkflowGraph } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'
import {
  applyWorkflowEvent,
  isWorkflowRunEvent,
  workflowGraphKey,
} from './graph-model'

/**
 * The workflow's step graph with live statuses: `workflow://*` events patch
 * the cached graph at once, then refetch it for the authoritative state.
 */
export function useWorkflowGraph(workflowId: string) {
  const queryClient = useQueryClient()

  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (!isWorkflowRunEvent(event)) return
      if (event.payload.workflowId !== workflowId) return
      const key = workflowGraphKey(workflowId)
      queryClient.setQueryData<WorkflowGraph>(key, graph =>
        graph ? applyWorkflowEvent(graph, event) : graph
      )
      void queryClient.invalidateQueries({ queryKey: key })
    })
    return () => subscription.unsubscribe()
  }, [queryClient, workflowId])

  return useQuery({
    queryKey: workflowGraphKey(workflowId),
    queryFn: async () =>
      unwrapResult(await client.getWorkflowGraph(workflowId)),
    staleTime: 0,
  })
}
