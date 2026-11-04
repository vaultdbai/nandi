import { useMutation } from '@tanstack/react-query'
import { controlContainer, type ContainerAction } from '@/lib/agent-actions'
import type { ContainerState, ContainerStatus } from '@/lib/tauri-bindings'

/**
 * Container lifecycle for one agent: the status to show (a start in flight
 * shows as starting before Rust reports it) and the action mutation.
 */
export function useContainerControl(
  agentId: string,
  container: ContainerState
) {
  const mutation = useMutation({
    mutationFn: (action: ContainerAction) => controlContainer(agentId, action),
  })
  const busy = mutation.isPending
  const status: ContainerStatus =
    busy && mutation.variables !== 'stop' ? 'starting' : container.status
  return { status, busy, run: mutation.mutate }
}
