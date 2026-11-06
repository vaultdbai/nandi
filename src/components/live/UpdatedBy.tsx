import { useTranslation } from 'react-i18next'
import { useNow } from '@/components/agents/task-utils'
import { useLatestChange } from '@/hooks/useLiveChanges'
import type { ChangeArea } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'
import { cn } from '@/lib/utils'
import { updatedText } from './live-model'

/** The clock behind "just now" only needs to move once a minute or so */
const TICK_MS = 30_000

/**
 * "Updated just now by Chief of Staff": who last changed a note or an
 * agent, when the app saw the change. Without one, the file's own time
 * (`updatedAt`) if known, else nothing.
 */
export function UpdatedBy({
  areas,
  targetId,
  updatedAt,
  className,
}: {
  areas: readonly ChangeArea[]
  targetId: string
  /** The file's modification time, for changes from before this session */
  updatedAt?: number | null
  className?: string
}) {
  const { t, i18n } = useTranslation()
  const change = useLatestChange(areas, targetId)
  const now = useNow(true, TICK_MS)
  let text: string | null = null
  if (change) text = updatedText(change, now, t, i18n.language)
  else if (updatedAt)
    text = t('live.updatedAt', {
      when: formatRelativeTime(updatedAt, i18n.language, new Date(now)),
    })
  if (!text) return null
  return (
    <p
      className={cn('text-xs text-muted-foreground', className)}
      data-testid="updated-by"
    >
      {text}
    </p>
  )
}
