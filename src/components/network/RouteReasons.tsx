import { useTranslation } from 'react-i18next'
import type { RouteReason } from '@/lib/bindings'
import { reasonText } from '@/lib/route-reasons'
import { cn } from '@/lib/utils'

/**
 * Why the relay picked a provider, in the user's language: its reasons
 * (typed codes) as one short line. Never another member's success rate:
 * the relay doesn't send any.
 */
export function RouteReasons({
  reasons,
  className,
}: {
  reasons: RouteReason[]
  className?: string
}) {
  const { t, i18n } = useTranslation()
  if (reasons.length === 0) return null
  const formatDate = (ms: number) =>
    new Date(ms).toLocaleDateString(i18n.language, {
      weekday: 'long',
      month: 'short',
      day: 'numeric',
    })
  return (
    <span
      data-testid="route-reasons"
      className={cn('text-xs text-muted-foreground', className)}
    >
      {reasons
        .map(reason => {
          const { key, values } = reasonText(reason, formatDate)
          return t(key, values)
        })
        .join(' · ')}
    </span>
  )
}
