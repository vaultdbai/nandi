import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  Camera,
  Circle,
  CircleCheck,
  CircleMinus,
  CircleX,
  ListChecks,
} from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Spinner } from '@/components/ui/spinner'
import { useRunPlan, useRunScreenshot } from '@/hooks/useRunPlan'
import { client, isDesktop } from '@/lib/client'
import type { RunPlanStep, RunStepStatus } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import {
  fileName,
  isScreenshot,
  missesScreenshot,
  planProgress,
  STEP_STATUS_CLASSES,
} from './run-plan-model'

/**
 * A run's plan as a live checklist: each step, its status, its result and
 * its evidence (screenshots as thumbnails that open the file). Shown in
 * the chat, the intake thread and Run Detail; nothing when the run has no
 * plan.
 */
export function RunPlanCard({ runId }: { runId: string }) {
  const { t } = useTranslation()
  const { data: plan } = useRunPlan(runId)
  if (!plan) return null
  const { closed, total } = planProgress(plan)

  return (
    <section
      aria-label={t('runPlan.title')}
      className="rounded-lg border bg-card text-sm"
    >
      <header className="flex items-center gap-2 border-b px-3 py-2">
        <ListChecks aria-hidden className="size-4 text-muted-foreground" />
        <h3 className="text-xs font-medium">{t('runPlan.title')}</h3>
        {plan.revision > 1 && (
          <span className="text-xs text-muted-foreground">
            {t('runPlan.revision', { revision: plan.revision })}
          </span>
        )}
        <span className="ms-auto text-xs text-muted-foreground tabular-nums">
          {t('runPlan.progress', { closed, total })}
        </span>
      </header>
      <ol className="flex flex-col divide-y">
        {plan.steps.map((step, index) => (
          <PlanStepRow
            key={`${index}:${step.title}`}
            runId={runId}
            number={index + 1}
            step={step}
          />
        ))}
      </ol>
    </section>
  )
}

function StatusIcon({ status }: { status: RunStepStatus }) {
  const className = cn('size-4 shrink-0', STEP_STATUS_CLASSES[status])
  switch (status) {
    case 'pending':
      return <Circle aria-hidden className={className} />
    case 'doing':
      return <Spinner className={cn('size-4', STEP_STATUS_CLASSES.doing)} />
    case 'done':
      return <CircleCheck aria-hidden className={className} />
    case 'skipped':
      return <CircleMinus aria-hidden className={className} />
    case 'failed':
      return <CircleX aria-hidden className={className} />
  }
}

function PlanStepRow({
  runId,
  number,
  step,
}: {
  runId: string
  number: number
  step: RunPlanStep
}) {
  const { t } = useTranslation()
  const screenshots = step.files.filter(isScreenshot)
  const otherFiles = step.files.filter(file => !isScreenshot(file))

  return (
    <li
      className={cn(
        'flex gap-2 px-3 py-2',
        step.status === 'doing' && 'bg-info/5'
      )}
    >
      <span className="mt-0.5" title={t(`runPlan.status.${step.status}`)}>
        <StatusIcon status={step.status} />
        <span className="sr-only">{t(`runPlan.status.${step.status}`)}</span>
      </span>
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
          <span className="text-xs text-muted-foreground tabular-nums">
            {number}.
          </span>
          <span
            className={cn(
              'min-w-0 break-words',
              step.status === 'skipped' && 'text-muted-foreground line-through'
            )}
          >
            {step.title}
          </span>
          <Badge variant="outline" className="text-[10px]">
            {t(`runPlan.kind.${step.kind}`)}
          </Badge>
          {step.evidence === 'screenshot' && screenshots.length === 0 && (
            <span
              className={cn(
                'flex items-center gap-1 text-xs',
                missesScreenshot(step)
                  ? 'text-warning'
                  : 'text-muted-foreground'
              )}
            >
              <Camera aria-hidden className="size-3" />
              {missesScreenshot(step)
                ? t('runPlan.missingScreenshot')
                : t('runPlan.screenshotPlanned')}
            </span>
          )}
        </div>
        {step.note && (
          <p className="text-xs text-muted-foreground">{step.note}</p>
        )}
        {step.result && (
          <p className="text-xs break-words text-foreground/80">
            {step.result}
          </p>
        )}
        {screenshots.length > 0 && (
          <div className="flex flex-wrap gap-2 pt-1">
            {screenshots.map(path => (
              <Screenshot key={path} runId={runId} path={path} />
            ))}
          </div>
        )}
        {otherFiles.length > 0 && (
          <ul className="flex flex-wrap gap-2 text-xs">
            {otherFiles.map(path => (
              <li key={path}>
                <code dir="ltr" className="font-mono">
                  {path}
                </code>
              </li>
            ))}
          </ul>
        )}
      </div>
    </li>
  )
}

/** A thumbnail that opens the file (desktop); gone after 30 days */
function Screenshot({ runId, path }: { runId: string; path: string }) {
  const { t } = useTranslation()
  const { data: src, isPending } = useRunScreenshot(runId, path)
  const name = fileName(path)

  const open = async () => {
    const result = await client.openRunFile(runId, path, false)
    if (result.status === 'error') toast.error(result.error)
  }

  if (isPending) {
    return <div className="h-16 w-24 animate-pulse rounded-md bg-muted" />
  }
  if (!src) {
    return (
      <span className="flex h-16 w-24 items-center justify-center rounded-md border border-dashed px-1 text-center text-[10px] text-muted-foreground">
        {t('runPlan.screenshotGone')}
      </span>
    )
  }
  const image = (
    <img
      src={src}
      alt={t('runPlan.screenshotAlt', { name })}
      className="h-16 w-24 rounded-md border object-cover object-top"
    />
  )
  return isDesktop() ? (
    <button
      type="button"
      onClick={() => void open()}
      title={t('runPlan.openScreenshot', { name })}
      className="rounded-md focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
    >
      {image}
    </button>
  ) : (
    image
  )
}
