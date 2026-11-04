import { useTranslation } from 'react-i18next'
import type { WorkItem } from '@/lib/bindings'
import { cn } from '@/lib/utils'

/** A progress bar with its step count (Home lists work items itself) */
export function WorkProgressBar({
  item,
  className,
}: {
  item: WorkItem
  className?: string
}) {
  const { t } = useTranslation()
  const { completedSteps, totalSteps, percent } = item.progress
  return (
    <div className={cn('flex items-center gap-2', className)}>
      <div
        role="progressbar"
        aria-label={t('work.progress')}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted"
      >
        <div
          className={cn(
            'h-full rounded-full',
            item.state === 'failed' ? 'bg-destructive' : 'bg-primary'
          )}
          style={{ width: `${percent}%` }}
        />
      </div>
      <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
        {t('work.steps', { done: completedSteps, total: totalSteps })}
      </span>
    </div>
  )
}
