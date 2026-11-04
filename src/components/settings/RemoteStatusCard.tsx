import { useTranslation } from 'react-i18next'
import { ArrowDown, ArrowUp, GitBranch, Globe } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import type { RemoteStatus } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'

export interface RemoteStatusCardProps {
  status: RemoteStatus
}

/** Whether the remote is configured and reachable, its branch, and sync state */
export function RemoteStatusCard({ status }: RemoteStatusCardProps) {
  const { t, i18n } = useTranslation()
  const config = status.config
  const when = (at: number | null | undefined) =>
    at ? formatRelativeTime(at, i18n.language) : t('remote.card.never')

  return (
    <section
      className="space-y-3 rounded-lg border p-4"
      data-testid="remote-status-card"
    >
      <div className="flex items-center gap-2">
        <Globe className="size-5 shrink-0 text-muted-foreground" />
        <p
          className="min-w-0 flex-1 truncate font-mono text-sm"
          dir="ltr"
          title={config?.url}
        >
          {config?.url ?? t('remote.empty.title')}
        </p>
        <Badge variant={status.reachable ? 'secondary' : 'destructive'}>
          {status.reachable
            ? t('remote.card.reachable')
            : t('remote.card.unreachable')}
        </Badge>
      </div>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-sm">
        <dt className="text-muted-foreground">{t('remote.card.branch')}</dt>
        <dd className="flex items-center gap-1.5">
          <GitBranch className="size-3.5 text-muted-foreground" />
          <code dir="ltr" className="font-mono">
            {status.targetBranch}
          </code>
        </dd>
        <dt className="text-muted-foreground">{t('remote.card.sync')}</dt>
        <dd className="flex items-center gap-3 tabular-nums">
          <span
            className="flex items-center gap-1"
            title={t('remote.card.ahead', { count: status.ahead })}
          >
            <ArrowUp className="size-3.5" />
            {status.ahead}
          </span>
          <span
            className="flex items-center gap-1"
            title={t('remote.card.behind', { count: status.behind })}
          >
            <ArrowDown className="size-3.5" />
            {status.behind}
          </span>
          <span className="sr-only">
            {t('remote.card.ahead', { count: status.ahead })},{' '}
            {t('remote.card.behind', { count: status.behind })}
          </span>
        </dd>
        <dt className="text-muted-foreground">{t('remote.card.lastPush')}</dt>
        <dd>{when(config?.lastPushAt)}</dd>
        <dt className="text-muted-foreground">{t('remote.card.lastPull')}</dt>
        <dd>{when(config?.lastPullAt)}</dd>
        <dt className="text-muted-foreground">{t('remote.card.credential')}</dt>
        <dd>
          {t(`remote.form.credential.${config?.credentialKind ?? 'none'}`)}
        </dd>
      </dl>
    </section>
  )
}
