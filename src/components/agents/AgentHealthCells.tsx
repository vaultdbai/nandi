import { useTranslation } from 'react-i18next'
import type { AgentDay } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { dayTone, type DayTone } from './agent-page-model'

const TONE_CLASSES: Record<DayTone, string> = {
  none: 'border border-border bg-transparent',
  ok: 'bg-success',
  some: 'bg-warning',
  all: 'bg-destructive',
}

/** Seven day-cells, oldest first: all completed, some failed, all failed, none */
export function AgentHealthCells({ days }: { days: AgentDay[] }) {
  const { t } = useTranslation()
  return (
    <span
      role="img"
      aria-label={t('agentPage.health.cellsLabel')}
      className="inline-flex shrink-0 gap-0.5"
    >
      {days.map((day, index) => (
        <span
          // Days have no identity beyond their place in the week
          key={index}
          title={t('agentPage.health.day', {
            completed: day.completed,
            failed: day.failed,
          })}
          className={cn('h-3 w-1.5 rounded-sm', TONE_CLASSES[dayTone(day)])}
        />
      ))}
    </span>
  )
}
