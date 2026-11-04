import { useTranslation } from 'react-i18next'
import { Bot, Users } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { MemberAvatar } from '@/components/network/MemberAvatar'
import type { RosterMember } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'

export interface MemberCardProps {
  teamId: string
  member: RosterMember
}

/** One roster member of a team: presence, what they shared, delegations with them */
export function MemberCard({ teamId, member }: MemberCardProps) {
  const { t } = useTranslation()
  const { delegationStats: stats } = member
  const instanceId = member.instanceId

  return (
    <Card
      data-testid={`roster-member-${member.handle}`}
      data-team={teamId}
      className="gap-3"
    >
      <CardHeader className="flex flex-row items-center gap-3">
        <MemberAvatar
          name={member.displayName}
          online={member.gateway ? member.online : undefined}
        />
        <div className="min-w-0">
          <CardTitle className="truncate">{member.displayName}</CardTitle>
          <CardDescription className="truncate" dir="ltr">
            @{member.handle}
          </CardDescription>
        </div>
      </CardHeader>
      <CardContent className="flex flex-col gap-2 text-sm">
        {!member.gateway && (
          <p className="text-xs text-muted-foreground">
            {t('members.notConnected')}
          </p>
        )}
        <div className="flex flex-wrap gap-x-4 gap-y-1 text-xs text-muted-foreground">
          <span className="inline-flex items-center gap-1">
            <Bot className="size-3.5" aria-hidden />
            {t('members.roster.sharedAgents', {
              count: member.sharedAgents.length,
            })}
          </span>
          <span className="inline-flex items-center gap-1">
            <Users className="size-3.5" aria-hidden />
            {t('members.roster.teamCount', { count: member.teamCount })}
          </span>
        </div>
        {stats.sent + stats.received > 0 && (
          <div className="flex flex-wrap gap-1">
            <Badge variant="outline">
              {t('members.roster.sent', { count: stats.sent })}
            </Badge>
            <Badge variant="outline">
              {t('members.roster.received', { count: stats.received })}
            </Badge>
          </div>
        )}
      </CardContent>
      <CardFooter className="mt-auto">
        <Button
          size="sm"
          variant="outline"
          disabled={!instanceId}
          onClick={() => {
            if (instanceId) {
              useNavigationStore
                .getState()
                .navigate({ kind: 'member-profile', instanceId })
            }
          }}
        >
          {t('network.openProfile')}
        </Button>
      </CardFooter>
    </Card>
  )
}
