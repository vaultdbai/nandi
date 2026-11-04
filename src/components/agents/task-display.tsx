/** The task state pill shared by the status panels and the thread */
import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'
import type { TaskState } from '@/lib/bindings'
import { cn } from '@/lib/utils'

const STATE_TONE: Record<TaskState, string> = {
  submitted: 'bg-muted text-muted-foreground',
  delivered: 'bg-muted text-muted-foreground',
  acknowledged: 'bg-info/15 text-info',
  working: 'bg-info/15 text-info',
  inputRequired: 'bg-warning/15 text-warning',
  completed: 'bg-success/15 text-success',
  failed: 'bg-destructive/15 text-destructive',
  rejected: 'bg-destructive/15 text-destructive',
  cancelled: 'bg-muted text-muted-foreground',
  timedOut: 'bg-destructive/15 text-destructive',
  abandoned: 'bg-muted text-muted-foreground',
  reassigned: 'bg-muted text-muted-foreground',
}

export function TaskStatePill({
  state,
  className,
}: {
  state: TaskState
  className?: string
}) {
  const { t } = useTranslation()
  return (
    <Badge
      variant="outline"
      className={cn('border-transparent', STATE_TONE[state], className)}
      data-state={state}
    >
      {t(`tasks.state.${state}`)}
    </Badge>
  )
}
