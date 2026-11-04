import { ProviderStatePanel } from '@/components/agent-console/ProviderStatePanel'
import { TaskStatusPanel } from '@/components/agent-console/TaskStatusPanel'
import { isTerminalState, useTasks } from '@/hooks/useTasks'

/** How many in-flight tasks the strip shows at once */
const MAX_PANELS = 3

/**
 * The agent chat's in-flight delegations: a `TaskStatusPanel` for each
 * task the agent sent, a `ProviderStatePanel` for each it's serving.
 * Renders nothing when there are none (or the task service is down).
 *
 * Mount it under the chat header (`AgentView`'s `AgentChat`).
 */
export function AgentTaskStrip({ agentId }: { agentId: string }) {
  const { data: tasks } = useTasks({
    state: null,
    role: null,
    agentId,
    limit: 20,
  })

  const active = (tasks ?? [])
    .filter(task => !isTerminalState(task.state))
    .slice(0, MAX_PANELS)
  if (active.length === 0) return null

  return (
    <div data-testid="agent-task-strip">
      {active.map(task =>
        task.requesterAgent === agentId ? (
          <TaskStatusPanel key={task.id} taskId={task.id} />
        ) : (
          <ProviderStatePanel key={task.id} task={task} />
        )
      )}
    </div>
  )
}
