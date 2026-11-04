import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  AlertTriangle,
  CloudOff,
  ExternalLink,
  MessageCircleQuestion,
  RefreshCw,
  Send,
  X,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { TaskStatePill } from '@/components/agents/task-display'
import {
  useAgentLabel,
  taskStartedAt,
  useNow,
  useOfflineProvider,
} from '@/components/agents/task-utils'
import {
  isTerminalState,
  taskErrorMessage,
  useCancelTask,
  useQueryTaskStatus,
  useTask,
  useTaskPendingClarifications,
  useTaskProgress,
} from '@/hooks/useTasks'
import { formatDuration } from '@/lib/format-time'
import { toastPersonError } from '@/lib/toast-error'
import { useNavigationStore } from '@/store/navigation-store'

export interface TaskStatusPanelProps {
  taskId: string
}

/**
 * The requester's strip for one in-flight task: who it went to, its state,
 * ETA and progress, with Query status and Cancel. Says when questions wait
 * for the user or the task is queued for an offline member, warns once the
 * task has run past twice its ETA; hides when the task ends.
 */
export function TaskStatusPanel({ taskId }: TaskStatusPanelProps) {
  const { t } = useTranslation()
  const agentLabel = useAgentLabel()
  const { data: task } = useTask(taskId)
  const { data: progress } = useTaskProgress(taskId)
  const { data: pending } = useTaskPendingClarifications(taskId)
  const offlineProvider = useOfflineProvider(task)
  const active = task !== undefined && !isTerminalState(task.state)
  const now = useNow(active)
  const query = useQueryTaskStatus()
  const cancel = useCancelTask()

  if (!task || !active) return null

  const elapsedSeconds = Math.max(0, (now - taskStartedAt(task)) / 1000)
  const eta = task.etaSeconds
  const overdue = eta !== null && eta > 0 && elapsedSeconds > 2 * eta
  const percent = progress?.percent ?? null
  const pendingCount = pending?.length ?? 0
  const openThread = () =>
    useNavigationStore
      .getState()
      .navigate({ kind: 'delegation-thread', taskId: task.id })

  const queryStatus = () =>
    query.mutate(task.id, {
      onError: error =>
        toastPersonError(error, failure =>
          toast.error(
            t('tasks.panel.queryFailed', { error: taskErrorMessage(failure) })
          )
        ),
    })

  const cancelTask = () =>
    cancel.mutate(
      { taskId: task.id },
      {
        onSuccess: () => toast.success(t('tasks.panel.cancelled')),
        onError: error =>
          toast.error(
            t('tasks.panel.cancelFailed', { error: taskErrorMessage(error) })
          ),
      }
    )

  return (
    <div
      className="flex shrink-0 flex-col gap-1.5 border-b bg-muted/30 px-4 py-2 text-sm"
      data-testid="task-status-panel"
      role="status"
    >
      <div className="flex flex-wrap items-center gap-2">
        <Send className="size-4 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate">
          {t('tasks.panel.delegating', {
            provider: agentLabel(task.providerAgent, task.providerInstance),
            capability: task.capability,
          })}
        </span>
        <TaskStatePill state={task.state} />
        <span className="text-xs tabular-nums text-muted-foreground">
          {eta !== null
            ? t('tasks.panel.eta', { eta: formatDuration(eta * 1000) })
            : t('tasks.panel.noEta')}
          {' · '}
          {t('tasks.panel.elapsed', {
            elapsed: formatDuration(elapsedSeconds * 1000),
          })}
        </span>
        <Button
          variant="ghost"
          size="sm"
          disabled={query.isPending}
          onClick={queryStatus}
        >
          {query.isPending ? <Spinner /> : <RefreshCw />}
          {t('tasks.panel.query')}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          disabled={cancel.isPending}
          onClick={cancelTask}
        >
          <X />
          {t('tasks.panel.cancel')}
        </Button>
        <Button
          variant="ghost"
          size="icon-sm"
          aria-label={t('tasks.panel.openThread')}
          title={t('tasks.panel.openThread')}
          onClick={openThread}
        >
          <ExternalLink />
        </Button>
      </div>

      {percent !== null && (
        <div
          role="progressbar"
          aria-label={t('tasks.panel.progress')}
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={percent}
          className="h-1.5 w-full overflow-hidden rounded-full bg-muted"
        >
          <div
            className="h-full rounded-full bg-primary transition-[width]"
            style={{ width: `${Math.min(100, Math.max(0, percent))}%` }}
          />
        </div>
      )}
      {progress?.note && (
        <p className="truncate text-xs text-muted-foreground">
          {progress.note}
        </p>
      )}
      {query.data && (
        <p
          className="text-xs text-muted-foreground"
          data-testid="task-status-report"
        >
          {t(
            query.data.live
              ? 'tasks.panel.reportLive'
              : 'tasks.panel.reportLocal',
            {
              state: t(`tasks.state.${query.data.state}`),
              elapsed: formatDuration(query.data.elapsedSeconds * 1000),
            }
          )}
        </p>
      )}
      {pendingCount > 0 && (
        <p
          className="flex items-center gap-1.5 text-xs text-warning"
          data-testid="task-needs-answer"
        >
          <MessageCircleQuestion className="size-3.5 shrink-0" />
          <span className="flex-1">
            {t('tasks.panel.needsAnswer', { count: pendingCount })}
          </span>
          <Button
            variant="link"
            size="sm"
            className="h-auto p-0 text-xs"
            onClick={openThread}
          >
            {t('tasks.panel.answer')}
          </Button>
        </p>
      )}
      {offlineProvider && (
        <p
          className="flex items-center gap-1.5 text-xs text-muted-foreground"
          data-testid="task-queued-offline"
        >
          <CloudOff className="size-3.5 shrink-0" />
          {t('tasks.queuedOffline', { name: offlineProvider })}
        </p>
      )}
      {overdue && (
        <p className="flex items-center gap-1.5 text-xs text-warning">
          <AlertTriangle className="size-3.5 shrink-0" />
          {t('tasks.panel.overdue')}
        </p>
      )}
    </div>
  )
}
