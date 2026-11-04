import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronRight } from 'lucide-react'
import { RunTimelineList } from '@/components/agent-console/RunTimeline'
import { timelineSteps } from '@/components/agent-console/run-detail-model'
import { TaskStatePill } from '@/components/agents/task-display'
import { Button } from '@/components/ui/button'
import { useRunDetail } from '@/hooks/useSnapshots'
import type { Request, RequestStep } from '@/lib/bindings'
import { formatDuration } from '@/lib/format-time'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import { STEP_MARKERS, stepDuration, stepNumbers } from './request-model'

/** Timeline rows shown inside an expanded agent step */
const MINI_TIMELINE_ROWS = 5

const navigate = useNavigationStore.getState().navigate

/** The step rail: system steps small and muted, plan steps numbered */
export function RequestSteps({
  request,
  now,
  focused,
  onComplete,
}: {
  request: Request
  now: number
  /** The step to open with (the banner's, or the one asked for) */
  focused: string | null
  /** Marks a step waiting on the user done (local asks) */
  onComplete?: (step: RequestStep) => void
}) {
  const { t } = useTranslation()
  const numbers = stepNumbers(request.steps)
  const [open, setOpen] = useState<Set<string>>(
    () => new Set(focused ? [focused] : [])
  )
  const toggle = (id: string) =>
    setOpen(current => {
      const next = new Set(current)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })

  return (
    <ol aria-label={t('request.tabs.steps')} className="flex flex-col gap-1">
      {request.steps.map(step => (
        <StepRow
          key={step.id}
          request={request}
          step={step}
          number={numbers.get(step.id)}
          now={now}
          open={open.has(step.id)}
          highlighted={request.stuck?.stepId === step.id}
          onToggle={() => toggle(step.id)}
          onComplete={onComplete}
        />
      ))}
    </ol>
  )
}

function stepLabel(
  t: ReturnType<typeof useTranslation>['t'],
  request: Request,
  step: RequestStep
): string {
  if (step.kind !== 'system' || !step.stage) return step.label
  return t(`request.stage.${step.stage}`, {
    name: step.label || request.counterpart || t('request.someone'),
  })
}

function StepRow({
  request,
  step,
  number,
  now,
  open,
  highlighted,
  onToggle,
  onComplete,
}: {
  request: Request
  step: RequestStep
  number: number | undefined
  now: number
  open: boolean
  highlighted: boolean
  onToggle: () => void
  onComplete?: (step: RequestStep) => void
}) {
  const { t } = useTranslation()
  const system = step.kind === 'system'
  const duration = stepDuration(step, now)
  const expandable =
    !system ||
    step.detail !== null ||
    (highlighted && request.stuck?.reason != null)

  return (
    <li
      data-testid={`step-${step.id}`}
      data-status={step.status}
      className={cn(
        'rounded-md',
        highlighted && 'bg-warning/5 ring-1 ring-warning/30'
      )}
    >
      <button
        type="button"
        disabled={!expandable}
        aria-expanded={expandable ? open : undefined}
        onClick={onToggle}
        className={cn(
          'flex w-full items-center gap-3 rounded-md px-2 py-1.5 text-start enabled:hover:bg-accent/40',
          system ? 'text-xs text-muted-foreground' : 'text-sm'
        )}
      >
        <span
          aria-label={t(`request.stepStatus.${step.status}`)}
          className={cn(
            'w-4 shrink-0 text-center',
            step.status === 'done' && 'text-success',
            step.status === 'failed' && 'text-destructive',
            step.status === 'running' && 'animate-pulse text-info',
            step.status === 'waiting' && 'text-warning'
          )}
        >
          {STEP_MARKERS[step.status]}
        </span>
        {number !== undefined && (
          <span className="w-4 shrink-0 text-xs text-muted-foreground tabular-nums">
            {number}
          </span>
        )}
        <span
          className={cn('min-w-0 flex-1 truncate', !system && 'font-medium')}
        >
          {stepLabel(t, request, step)}
        </span>
        {!system && (
          <span className="shrink-0 text-xs text-muted-foreground">
            {t(`request.kind.${step.kind}`)}
            {step.progress && ` · ${step.progress}`}
          </span>
        )}
        {duration !== null && step.status !== 'pending' && (
          <span className="w-16 shrink-0 text-end text-xs text-muted-foreground tabular-nums">
            {formatDuration(duration)}
          </span>
        )}
        {expandable && (
          <ChevronRight
            aria-hidden
            className={cn(
              'size-4 shrink-0 transition-transform rtl:rotate-180',
              open && 'rotate-90 rtl:rotate-90'
            )}
          />
        )}
      </button>
      {onComplete && step.kind === 'person' && step.status === 'waiting' && (
        <div className="ms-9 mb-1">
          <Button size="sm" variant="outline" onClick={() => onComplete(step)}>
            {t('request.markDone')}
          </Button>
        </div>
      )}
      {open && expandable && (
        <div className="ms-9 mb-2 flex flex-col gap-2 border-s ps-3 text-sm">
          {step.detail && (
            <p className="text-xs text-muted-foreground">{step.detail}</p>
          )}
          {step.etaSeconds !== null && step.status === 'running' && (
            <p className="text-xs text-muted-foreground">
              {t('request.eta', {
                duration: formatDuration(step.etaSeconds * 1000),
              })}
            </p>
          )}
          {step.kind === 'agent' && step.link.runId && (
            <AgentStepRun runId={step.link.runId} />
          )}
          {step.attempts.length > 0 && <Attempts step={step} />}
          {step.children.length > 0 && (
            <ul
              aria-label={t('request.workflowSteps')}
              className="flex flex-col gap-0.5"
            >
              {step.children.map(child => (
                <li key={child.id} className="flex items-center gap-2 text-xs">
                  <span className="w-4 text-center">
                    {STEP_MARKERS[child.status]}
                  </span>
                  <span>{child.label}</span>
                </li>
              ))}
            </ul>
          )}
          {step.kind === 'peer' && step.link.taskId && (
            <Button
              size="sm"
              variant="outline"
              className="self-start"
              onClick={() =>
                navigate({
                  kind: 'delegation-thread',
                  taskId: step.link.taskId ?? '',
                })
              }
            >
              {t('request.openConversation')}
            </Button>
          )}
          {highlighted && request.stuck?.reason && (
            <details className="text-xs text-muted-foreground">
              <summary className="cursor-pointer">
                {t('request.rawReason')}
              </summary>
              <p dir="auto" className="mt-1 whitespace-pre-wrap">
                {request.stuck.reason}
              </p>
            </details>
          )}
        </div>
      )}
    </li>
  )
}

/** The last few events of the run behind an agent step */
function AgentStepRun({ runId }: { runId: string }) {
  const { t } = useTranslation()
  const { data: detail } = useRunDetail(runId)
  return (
    <div className="flex flex-col gap-2">
      {detail && (
        <RunTimelineList
          steps={timelineSteps(detail).slice(-MINI_TIMELINE_ROWS)}
        />
      )}
      <Button
        size="sm"
        variant="outline"
        className="self-start"
        onClick={() => navigate({ kind: 'run-detail', runId })}
      >
        {t('request.openRun')}
      </Button>
    </div>
  )
}

function Attempts({ step }: { step: RequestStep }) {
  const { t } = useTranslation()
  return (
    <ol aria-label={t('request.attempts')} className="flex flex-col gap-1">
      {step.attempts.map(attempt => (
        <li
          key={attempt.taskId}
          className="flex flex-wrap items-center gap-2 text-xs"
        >
          <span className="font-medium">
            {t('request.attempt', { n: attempt.attemptNumber })}
          </span>
          <span>{attempt.provider}</span>
          <TaskStatePill state={attempt.state} />
          {attempt.endedAt !== null && (
            <span className="text-muted-foreground tabular-nums">
              {formatDuration(attempt.endedAt - attempt.startedAt)}
            </span>
          )}
        </li>
      ))}
    </ol>
  )
}
