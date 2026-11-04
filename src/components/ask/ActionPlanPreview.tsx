import { useTranslation } from 'react-i18next'
import {
  Bot,
  Boxes,
  CircleSlash,
  Hand,
  TriangleAlert,
  User,
  Workflow,
} from 'lucide-react'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import type { ActionPlan, PlanStepKind } from '@/lib/bindings'
import { formatDuration } from '@/lib/format-time'
import { formatCostUsd } from '@/lib/format-usage'

const STEP_ICONS: Record<PlanStepKind, typeof Bot> = {
  localAgent: Bot,
  capability: Boxes,
  workflow: Workflow,
  human: User,
  manual: Hand,
}

/**
 * A plan card: the steps the ask would take, what it should cost and how
 * long it should take, warnings, and Run / Dismiss. A plan that was
 * rejected shows why instead.
 */
export function ActionPlanPreview({
  plan,
  busy = false,
  onApprove,
  onDismiss,
}: {
  plan: ActionPlan
  busy?: boolean
  onApprove?: () => void
  onDismiss?: () => void
}) {
  const { t, i18n } = useTranslation()

  if (plan.rejection) {
    return (
      <Alert>
        <CircleSlash />
        <AlertDescription>{plan.rejection}</AlertDescription>
      </Alert>
    )
  }

  const estimates = [
    plan.estimatedDurationSeconds != null &&
      t('ask.plan.duration', {
        duration: formatDuration(plan.estimatedDurationSeconds * 1000),
      }),
    plan.estimatedCostUsd != null &&
      t('ask.plan.cost', {
        cost: formatCostUsd(plan.estimatedCostUsd, i18n.language),
      }),
  ].filter(Boolean)

  return (
    <div className="flex flex-col gap-3">
      <ol aria-label={t('ask.plan.steps')} className="flex flex-col gap-1.5">
        {plan.steps.map((step, index) => {
          const Icon = STEP_ICONS[step.kind]
          return (
            <li
              key={`${index}-${step.target}`}
              className="flex items-start gap-2 text-sm"
            >
              <span className="mt-0.5 flex size-5 shrink-0 items-center justify-center rounded-full bg-muted text-xs text-muted-foreground">
                {index + 1}
              </span>
              <Icon
                aria-hidden
                className="mt-0.5 size-4 shrink-0 text-muted-foreground"
              />
              <span className="min-w-0 flex-1">
                {step.description}
                <span className="ms-1.5 font-mono text-xs text-muted-foreground">
                  {step.target}
                </span>
              </span>
            </li>
          )
        })}
      </ol>
      {estimates.length > 0 && (
        <p className="text-xs text-muted-foreground">{estimates.join(' · ')}</p>
      )}
      {plan.warnings.length > 0 && (
        <ul aria-label={t('ask.plan.warnings')} className="flex flex-col gap-1">
          {plan.warnings.map(warning => (
            <li
              key={warning}
              className="flex items-start gap-2 text-xs text-warning"
            >
              <TriangleAlert aria-hidden className="mt-0.5 size-3.5 shrink-0" />
              {warning}
            </li>
          ))}
        </ul>
      )}
      {(onApprove ?? onDismiss) && (
        <div className="flex justify-end gap-2">
          {onDismiss && (
            <Button size="sm" variant="ghost" onClick={onDismiss}>
              {t('ask.dismiss')}
            </Button>
          )}
          {onApprove && (
            <Button size="sm" onClick={onApprove} disabled={busy}>
              {t('ask.plan.run')}
            </Button>
          )}
        </div>
      )}
    </div>
  )
}
