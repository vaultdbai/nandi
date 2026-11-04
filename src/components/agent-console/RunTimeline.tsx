/**
 * A run's steps in order, with status dots: green check (done), blue pulse
 * (running), grey (pending), red (failed). The rail turns green behind
 * finished steps.
 */
import { useTranslation } from 'react-i18next'
import { Check, X } from 'lucide-react'
import { useRunDetail } from '@/hooks/useSnapshots'
import { formatDuration } from '@/lib/format-time'
import { cn } from '@/lib/utils'
import { toolLabel } from './tool-label'
import {
  timelineSteps,
  type StepState,
  type TimelineStep,
} from './run-detail-model'

function StatusDot({ state }: { state: StepState }) {
  const { t } = useTranslation()
  const label = t(`runDetail.timeline.state.${state}`)
  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={cn(
        'relative z-10 flex size-6 shrink-0 items-center justify-center rounded-full ring-4 ring-background',
        state === 'done' && 'bg-success text-background',
        state === 'failed' && 'bg-destructive text-background',
        state === 'pending' &&
          'border-2 border-muted-foreground/40 bg-background',
        state === 'running' && 'bg-info/20'
      )}
    >
      {state === 'done' && <Check className="size-3.5" strokeWidth={3} />}
      {state === 'failed' && <X className="size-3.5" strokeWidth={3} />}
      {state === 'running' && (
        <>
          <span className="absolute inline-flex size-full animate-ping rounded-full bg-info/50" />
          <span className="relative size-2.5 rounded-full bg-info" />
        </>
      )}
    </span>
  )
}

function stepTitle(
  t: ReturnType<typeof useTranslation>['t'],
  step: TimelineStep
): string {
  if (step.kind === 'finished' || step.kind === 'running') {
    return t(`runDetail.timeline.step.${step.kind}`, {
      status: t(`runs.status.${step.status ?? 'completed'}`),
    })
  }
  // What the tool does, as on the chat's tool cards
  const tool = step.tool ? toolLabel(step.tool, null, t) : ''
  return t(`runDetail.timeline.step.${step.kind}`, { tool })
}

/** A run's timeline (loads the run) */
export function RunTimeline({ runId }: { runId: string }) {
  const { data: detail } = useRunDetail(runId)
  if (!detail) return null
  return <RunTimelineList steps={timelineSteps(detail)} />
}

/** The rail itself, for any list of steps (Run Detail, a Request's step) */
export function RunTimelineList({ steps }: { steps: TimelineStep[] }) {
  const { t, i18n } = useTranslation()
  return (
    <ol aria-label={t('runDetail.tabs.timeline')} className="flex flex-col">
      {steps.map((step, index) => (
        <li key={`${step.id}-${index}`} className="relative flex gap-3 pb-6">
          {index < steps.length - 1 && (
            <span
              aria-hidden
              className={cn(
                'absolute start-3 top-6 bottom-0 w-0.5 -translate-x-1/2 rounded-full rtl:translate-x-1/2',
                step.state === 'done' ? 'bg-success/50' : 'bg-border'
              )}
            />
          )}
          <StatusDot state={step.state} />
          <time
            dateTime={new Date(step.at).toISOString()}
            className="w-16 shrink-0 pt-1 text-xs font-medium text-muted-foreground tabular-nums"
          >
            {new Date(step.at).toLocaleTimeString(i18n.language, {
              hour: '2-digit',
              minute: '2-digit',
              second: '2-digit',
            })}
          </time>
          <div className="min-w-0 flex-1 pt-0.5">
            <div className="flex flex-wrap items-baseline gap-x-2 text-sm font-semibold">
              {stepTitle(t, step)}
              {step.durationMs !== undefined && (
                <span className="text-xs font-normal text-muted-foreground tabular-nums">
                  {formatDuration(step.durationMs)}
                </span>
              )}
            </div>
            {step.description && (
              <div className="text-xs break-words whitespace-pre-wrap text-muted-foreground">
                {step.description}
              </div>
            )}
          </div>
        </li>
      ))}
    </ol>
  )
}
