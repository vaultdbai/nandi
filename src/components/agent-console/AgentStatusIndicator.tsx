import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'
import { cn } from '@/lib/utils'
import type { AgentStatus } from '@/types/agent'
import { AGENT_STATUS_STYLES } from './agent-status'

interface AgentStatusDotProps {
  status: AgentStatus
  className?: string
}

export function AgentStatusDot({ status, className }: AgentStatusDotProps) {
  const { t } = useTranslation()
  const style = AGENT_STATUS_STYLES[status]

  return (
    <span
      role="img"
      aria-label={t(style.labelKey)}
      className={cn('relative flex size-2.5 shrink-0', className)}
    >
      {status === 'running' && (
        <span
          className={cn(
            'absolute inline-flex size-full animate-ping rounded-full opacity-60',
            style.dotClassName
          )}
        />
      )}
      <span
        className={cn(
          'relative inline-flex size-full rounded-full',
          style.dotClassName
        )}
      />
    </span>
  )
}

interface AgentStatusPillProps {
  status: AgentStatus
  className?: string
}

export function AgentStatusPill({ status, className }: AgentStatusPillProps) {
  const { t } = useTranslation()
  const style = AGENT_STATUS_STYLES[status]

  return (
    <Badge
      variant="outline"
      className={cn('gap-1.5 rounded-full', style.pillClassName, className)}
    >
      <span
        aria-hidden
        className={cn('size-1.5 rounded-full', style.dotClassName)}
      />
      {t(style.labelKey)}
    </Badge>
  )
}
