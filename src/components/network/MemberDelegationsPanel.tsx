import { useTranslation } from 'react-i18next'
import { ArrowDownLeft, ArrowUpRight, History } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import type { DelegationStats, MemberProfile, TaskState } from '@/lib/bindings'
import { formatDuration, formatRelativeTime } from '@/lib/format-time'
import { useNavigationStore } from '@/store/navigation-store'

export interface MemberDelegationsPanelProps {
  profile: MemberProfile
}

const STATE_VARIANT: Partial<
  Record<TaskState, 'secondary' | 'destructive' | 'outline'>
> = {
  completed: 'secondary',
  failed: 'destructive',
  rejected: 'destructive',
  timedOut: 'destructive',
  abandoned: 'destructive',
}

/** Member profile: delegations to and from the member (stats, then the newest tasks) */
export function MemberDelegationsPanel({
  profile,
}: MemberDelegationsPanelProps) {
  const { t, i18n } = useTranslation()
  const tasks = profile.recentDelegations

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <History className="size-4" aria-hidden />
          {t('members.delegations.title')}
        </CardTitle>
        <CardDescription>
          {profile.delegationStats.lastAt === null
            ? t('members.delegations.never')
            : t('members.delegations.lastAt', {
                time: formatRelativeTime(
                  profile.delegationStats.lastAt,
                  i18n.language
                ),
              })}
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        <DelegationStatsRow stats={profile.delegationStats} />
        {tasks.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t('members.delegations.empty')}
          </p>
        ) : (
          <ul className="flex flex-col divide-y rounded-md border">
            {tasks.map(task => {
              const received = task.requesterInstance === profile.instanceId
              return (
                <li key={task.id}>
                  <button
                    type="button"
                    data-testid={`member-task-${task.id}`}
                    className="flex w-full items-center gap-3 px-3 py-2 text-start text-sm hover:bg-accent"
                    onClick={() =>
                      useNavigationStore.getState().navigate({
                        kind: 'delegation-thread',
                        taskId: task.id,
                      })
                    }
                  >
                    {received ? (
                      <ArrowDownLeft
                        className="size-4 shrink-0 text-muted-foreground"
                        aria-label={t('members.delegations.received')}
                      />
                    ) : (
                      <ArrowUpRight
                        className="size-4 shrink-0 text-muted-foreground"
                        aria-label={t('members.delegations.sent')}
                      />
                    )}
                    <span className="min-w-0 flex-1 truncate font-mono text-xs">
                      {task.capability}
                    </span>
                    <Badge variant={STATE_VARIANT[task.state] ?? 'outline'}>
                      {t(`members.taskState.${task.state}`)}
                    </Badge>
                    <span className="shrink-0 text-xs text-muted-foreground">
                      {formatRelativeTime(task.createdAt, i18n.language)}
                    </span>
                  </button>
                </li>
              )
            })}
          </ul>
        )}
      </CardContent>
    </Card>
  )
}

/** Sent / received / succeeded / failed / average duration */
export function DelegationStatsRow({ stats }: { stats: DelegationStats }) {
  const { t } = useTranslation()
  const items: [string, string][] = [
    [t('members.delegations.stats.sent'), String(stats.sent)],
    [t('members.delegations.stats.received'), String(stats.received)],
    [t('members.delegations.stats.succeeded'), String(stats.succeeded)],
    [t('members.delegations.stats.failed'), String(stats.failed)],
    [
      t('members.delegations.stats.avgDuration'),
      stats.avgDurationMs === null ? '—' : formatDuration(stats.avgDurationMs),
    ],
  ]
  return (
    <dl className="grid grid-cols-3 gap-3 sm:grid-cols-5">
      {items.map(([label, value]) => (
        <div key={label} className="flex flex-col">
          <dt className="text-xs text-muted-foreground">{label}</dt>
          <dd className="text-sm font-medium tabular-nums">{value}</dd>
        </div>
      ))}
    </dl>
  )
}
