import { useTranslation } from 'react-i18next'
import { ExternalLink, Inbox, MessageCircleQuestion } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { TaskStatePill } from '@/components/agents/task-display'
import { useAgentLabel } from '@/components/agents/task-utils'
import {
  isTerminalState,
  useTaskMessages,
  useTaskPendingClarifications,
} from '@/hooks/useTasks'
import type { TaskRecord } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { DEFAULT_CLARIFICATION_POLICY } from '@/components/agents/clarification-policy'
import { ReassignMenu } from './ReassignMenu'

export interface ProviderStatePanelProps {
  task: TaskRecord
  /** The requester's round limit, when known (default: the policy default) */
  maxRounds?: number
}

/**
 * The provider's strip for one task it's serving: who asked, the round of
 * clarification it's on, and questions waiting for an answer. Hides when
 * the task ends.
 */
export function ProviderStatePanel({
  task,
  maxRounds = DEFAULT_CLARIFICATION_POLICY.maxRounds,
}: ProviderStatePanelProps) {
  const { t } = useTranslation()
  const agentLabel = useAgentLabel()
  const { data: messages } = useTaskMessages(task.id)
  const { data: pending } = useTaskPendingClarifications(task.id)

  if (isTerminalState(task.state)) return null

  const round =
    messages?.filter(m => m.kind === 'clarification_request').length ?? 0
  const pendingCount = pending?.length ?? 0

  return (
    <div
      className="flex shrink-0 flex-wrap items-center gap-2 border-b bg-muted/30 px-4 py-2 text-sm"
      data-testid="provider-state-panel"
      role="status"
    >
      <Inbox className="size-4 shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1 truncate">
        {t('tasks.provider.handling', {
          requester: agentLabel(task.requesterAgent, task.requesterInstance),
          capability: task.capability,
        })}
        {' — '}
        {t('tasks.provider.round', { round, max: maxRounds })}
      </span>
      <TaskStatePill state={task.state} />
      {pendingCount > 0 && (
        <Badge variant="secondary" className="gap-1">
          <MessageCircleQuestion className="size-3" />
          {t('tasks.provider.pending', { count: pendingCount })}
        </Badge>
      )}
      <ReassignMenu task={task} />
      {/* No command sends progress by hand yet; the executor reports it */}
      <Tooltip>
        <TooltipTrigger asChild>
          <span tabIndex={0}>
            <Button variant="ghost" size="sm" disabled>
              {t('tasks.provider.sendProgress')}
            </Button>
          </span>
        </TooltipTrigger>
        <TooltipContent>{t('tasks.provider.sendProgressHint')}</TooltipContent>
      </Tooltip>
      <Button
        variant="ghost"
        size="icon-sm"
        aria-label={t('tasks.panel.openThread')}
        title={t('tasks.panel.openThread')}
        onClick={() =>
          useNavigationStore
            .getState()
            .navigate({ kind: 'delegation-thread', taskId: task.id })
        }
      >
        <ExternalLink />
      </Button>
    </div>
  )
}
