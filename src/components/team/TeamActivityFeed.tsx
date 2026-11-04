import { useTranslation } from 'react-i18next'
import { Activity, ArrowDownLeft, ArrowUpRight } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { useTrustGraphHistory } from '@/hooks/useSharedAgents'
import { teamActivity } from '@/hooks/useTeams'
import type { AgentCard, Team } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'

/** Recent delegations within a team (from the delegation history) */
export function TeamActivityFeed({
  team,
  cards,
}: {
  team: Team
  cards: AgentCard[]
}) {
  const { t, i18n } = useTranslation()
  const history = useTrustGraphHistory()
  const entries = teamActivity(history.data ?? [], team, cards)

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Activity className="size-4" aria-hidden />
          {t('members.activity.title')}
        </CardTitle>
      </CardHeader>
      <CardContent>
        {entries.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t('members.activity.empty')}
          </p>
        ) : (
          <ul className="flex flex-col gap-2">
            {entries.map(entry => {
              const inbound = entry.direction === 'inbound'
              const who = inbound
                ? (entry.requesterOwner ?? entry.requesterAgent)
                : (entry.providerOwner ?? entry.providerAgent)
              return (
                <li
                  key={entry.taskId}
                  data-testid={`team-activity-${entry.taskId}`}
                  className="flex items-center gap-2 text-sm"
                >
                  {inbound ? (
                    <ArrowDownLeft
                      className="size-4 shrink-0 text-muted-foreground"
                      aria-hidden
                    />
                  ) : (
                    <ArrowUpRight
                      className="size-4 shrink-0 text-muted-foreground"
                      aria-hidden
                    />
                  )}
                  <span className="min-w-0 flex-1 truncate">
                    {inbound
                      ? t('members.activity.inbound', {
                          who,
                          capability: entry.capability,
                        })
                      : t('members.activity.outbound', {
                          who,
                          capability: entry.capability,
                        })}
                  </span>
                  <Badge variant="outline">
                    {t(`delegation.status.${entry.status}`)}
                  </Badge>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {formatRelativeTime(entry.startedAt, i18n.language)}
                  </span>
                </li>
              )
            })}
          </ul>
        )}
      </CardContent>
    </Card>
  )
}
