import { useTranslation } from 'react-i18next'
import { Square, TerminalSquare } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import {
  useHarnessSessions,
  useKillHarnessSession,
} from '@/hooks/useHarnessSessions'
import type { HarnessSessionStatus } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'

const STATUS_VARIANT: Record<
  HarnessSessionStatus,
  'default' | 'secondary' | 'destructive' | 'outline'
> = {
  starting: 'outline',
  running: 'default',
  finished: 'secondary',
  failed: 'destructive',
  killed: 'outline',
}

const isLive = (status: HarnessSessionStatus) =>
  status === 'starting' || status === 'running'

/**
 * Recent harness sessions (optionally one agent's): harness, status,
 * model, tokens, when, and Stop for live ones.
 */
export function HarnessSessionList({
  agentId = null,
}: {
  agentId?: string | null
}) {
  const { t, i18n } = useTranslation()
  const sessions = useHarnessSessions(agentId)
  const kill = useKillHarnessSession()

  if (sessions.isPending) {
    return <Skeleton className="h-24 w-full" />
  }
  const rows = sessions.data ?? []
  if (rows.length === 0) {
    return (
      <Empty>
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <TerminalSquare />
          </EmptyMedia>
          <EmptyTitle>{t('harness.sessions.emptyTitle')}</EmptyTitle>
          <EmptyDescription>
            {t('harness.sessions.emptyDescription')}
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  return (
    <ul className="flex flex-col divide-y rounded-lg border">
      {rows.map(session => (
        <li
          key={session.id}
          className="flex items-center gap-3 px-3 py-2 text-sm"
          data-testid="harness-session"
        >
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="font-medium">{session.adapterId}</span>
              <Badge variant={STATUS_VARIANT[session.status]}>
                {t(`harness.sessions.status.${session.status}`)}
              </Badge>
              {session.model && (
                <span className="truncate text-muted-foreground">
                  {session.model}
                </span>
              )}
            </div>
            <div className="text-xs text-muted-foreground">
              {session.agentId && <span>{session.agentId} · </span>}
              {formatRelativeTime(session.startedAt, i18n.language)}
              {' · '}
              {t('harness.sessions.tokens', {
                input: session.inputTokens,
                output: session.outputTokens,
              })}
            </div>
            {session.error && (
              <p className="truncate text-xs text-destructive">
                {session.error}
              </p>
            )}
          </div>
          {isLive(session.status) && (
            <Button
              size="sm"
              variant="outline"
              disabled={kill.isPending}
              onClick={() => kill.mutate(session.id)}
            >
              <Square />
              {t('harness.sessions.stop')}
            </Button>
          )}
        </li>
      ))}
    </ul>
  )
}
