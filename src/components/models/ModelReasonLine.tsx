import { useTranslation } from 'react-i18next'
import { TriangleAlert } from 'lucide-react'
import { modelLabel, type ModelReason } from '@/lib/models'
import { cn } from '@/lib/utils'

/**
 * Which model answers instead of the chosen one, and why (`modelReason`):
 * under the default model in Settings › Models, on an agent's page and in
 * its chat. Nothing when the chosen model answers.
 */
export function ModelReasonLine({
  reason,
  className,
}: {
  reason: ModelReason | null
  className?: string
}) {
  const { t } = useTranslation()
  if (!reason) return null
  return (
    <p
      role={reason.key === 'noModel' ? 'note' : undefined}
      className={cn(
        'flex items-center gap-1.5 text-sm',
        reason.key === 'noModel' ? 'text-warning' : 'text-muted-foreground',
        className
      )}
      data-testid="model-reason"
    >
      {reason.key === 'noModel' && (
        <TriangleAlert aria-hidden className="size-4 shrink-0" />
      )}
      {reason.key === 'noModel'
        ? t('models.noModel')
        : reason.key === 'noKeyFor'
          ? t('models.using.noKeyFor', {
              provider: t(`models.provider.${reason.provider}.name`),
              model: modelLabel(reason.model),
            })
          : t(`models.using.${reason.key}`, {
              model: modelLabel(reason.model),
            })}
    </p>
  )
}
