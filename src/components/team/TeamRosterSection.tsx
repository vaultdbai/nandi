import { useTranslation } from 'react-i18next'
import { Users } from 'lucide-react'
import { Skeleton } from '@/components/ui/skeleton'
import { useTeamRoster } from '@/hooks/useTeamRoster'
import { MemberCard } from './MemberCard'

export interface TeamRosterSectionProps {
  teamId: string
}

/** A team's roster (from `getTeamRoster`), as a grid of member cards */
export function TeamRosterSection({ teamId }: TeamRosterSectionProps) {
  const { t } = useTranslation()
  const roster = useTeamRoster(teamId)
  const members = roster.data?.members ?? []

  return (
    <section aria-labelledby={`roster-${teamId}`} className="grid gap-3">
      <h3
        id={`roster-${teamId}`}
        className="flex items-center gap-2 text-sm font-semibold"
      >
        <Users className="size-4" aria-hidden />
        {t('members.roster.title')}
      </h3>
      {roster.isPending && (
        <div className="grid gap-3 md:grid-cols-2">
          <Skeleton className="h-36" />
          <Skeleton className="h-36" />
        </div>
      )}
      {roster.error && (
        <p className="text-sm text-muted-foreground">
          {t('members.roster.unavailable', { error: roster.error.message })}
        </p>
      )}
      {roster.data && members.length === 0 && (
        <p className="text-sm text-muted-foreground">
          {t('members.roster.empty')}
        </p>
      )}
      {members.length > 0 && (
        <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          {members.map(member => (
            <MemberCard key={member.handle} teamId={teamId} member={member} />
          ))}
        </div>
      )}
    </section>
  )
}
