/**
 * Workflow run presentation: the segmented step-progress bar and the run
 * summary card.
 */
import { useTranslation } from 'react-i18next'
import { useNow } from '@/components/agents/task-utils'
import { formatDuration, formatRelativeTime } from '@/lib/format-time'
import type { WorkflowRun, WorkflowStepRun } from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'
import {
  RUN_WASH,
  runStatusStyle,
  STEP_STATUS_STYLES,
} from './run-status-styles'

/** One segment per step, coloured by its status */
export function StepSegments({
  steps,
  className,
}: {
  steps: WorkflowStepRun[]
  className?: string
}) {
  const { t } = useTranslation()
  return (
    <div
      className={cn('flex h-1.5 w-full gap-1', className)}
      role="img"
      aria-label={t('workflows.runSummary.progress', {
        done: steps.filter(s => s.status === 'done').length,
        total: steps.length,
      })}
    >
      {steps.map(step => (
        <span
          key={step.stepId}
          title={`${step.stepId} · ${t(`workflows.stepStatus.${step.status}`)}`}
          className={cn(
            'h-full min-w-1 flex-1 rounded-full transition-colors',
            STEP_STATUS_STYLES[step.status].fillClassName
          )}
        />
      ))}
    </div>
  )
}

/** The run at a glance: status, timing, progress, and per-status counts */
export function WorkflowRunSummary({ run }: { run: WorkflowRun }) {
  const { t, i18n } = useTranslation()
  const style = runStatusStyle(run.status)
  const Icon = style.icon
  const done = run.steps.filter(s => s.status === 'done').length
  const total = run.steps.length
  const now = useNow(run.endedAt === null)
  const elapsed = (run.endedAt ?? now) - run.startedAt
  const counts = (
    ['done', 'running', 'awaitingApproval', 'failed', 'pending'] as const
  )
    .map(status => ({
      status,
      count: run.steps.filter(
        s =>
          s.status === status ||
          (status === 'failed' && s.status === 'rejected')
      ).length,
    }))
    .filter(entry => entry.count > 0)

  return (
    <section
      aria-label={t('workflows.runSummary.label')}
      className={cn(
        'flex flex-col gap-4 rounded-2xl border bg-gradient-to-br to-card to-60% p-5 shadow-sm',
        RUN_WASH[run.status]
      )}
    >
      <div className="flex flex-wrap items-center gap-4">
        <span
          className={cn(
            'flex size-12 shrink-0 items-center justify-center rounded-xl',
            style.tileClassName
          )}
        >
          <Icon className={cn('size-6', style.iconClassName)} aria-hidden />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-base font-semibold">
              {t(`workflows.status.${run.status}`)}
            </span>
            <code
              dir="ltr"
              className="rounded bg-muted px-1.5 py-0.5 font-mono text-[11px] text-muted-foreground"
              title={run.id}
            >
              {run.id.slice(0, 8)}
            </code>
          </div>
          <div className="text-sm text-muted-foreground">
            {t('workflows.runSummary.started', {
              time: formatRelativeTime(run.startedAt, i18n.language),
            })}
            <span aria-hidden> · </span>
            {t(
              run.endedAt === null
                ? 'workflows.runSummary.runningFor'
                : 'workflows.runSummary.tookDuration',
              { duration: formatDuration(elapsed) }
            )}
          </div>
        </div>
        <div className="text-end">
          <div className="text-2xl font-semibold tabular-nums">
            {done}
            <span className="text-base text-muted-foreground">/{total}</span>
          </div>
          <div className="text-xs text-muted-foreground">
            {t('workflows.runSummary.stepsDone')}
          </div>
        </div>
      </div>

      <StepSegments steps={run.steps} className="h-2" />

      <ul className="flex flex-wrap gap-2">
        {counts.map(({ status, count }) => (
          <li
            key={status}
            className={cn(
              'flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs font-medium',
              STEP_STATUS_STYLES[status].pillClassName
            )}
          >
            <span
              aria-hidden
              className={cn(
                'size-1.5 rounded-full',
                STEP_STATUS_STYLES[status].fillClassName
              )}
            />
            {t(`workflows.runSummary.count.${status}`, { count })}
          </li>
        ))}
      </ul>
    </section>
  )
}
