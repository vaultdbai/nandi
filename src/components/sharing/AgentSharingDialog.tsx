import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { RadioTower, Users } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Label } from '@/components/ui/label'
import { Skeleton } from '@/components/ui/skeleton'
import { Switch } from '@/components/ui/switch'
import {
  useAgentSharing,
  useSetRelaySharing,
  useToggleTeamShare,
} from '@/hooks/useAgentSharing'
import {
  errorMessage,
  ownCard,
  useAvailableProviders,
} from '@/hooks/useDelegation'
import { formatRelativeTime } from '@/lib/format-time'
import { MANAGE_TEAMS, useNavigationStore } from '@/store/navigation-store'
import type { AgentInfo } from '@/types/agent'

/**
 * Who can see and call one of my agents: my teams (each team's private
 * registry) and the relay. Shows what others see of it: owner,
 * description, capabilities, and how much it's been called.
 */
export function AgentSharingDialog({
  agent,
  open,
  onOpenChange,
}: {
  agent: AgentInfo
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const { t } = useTranslation()

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {t('agentSharing.title', { name: agent.name })}
          </DialogTitle>
          <DialogDescription>{t('agentSharing.description')}</DialogDescription>
        </DialogHeader>
        {open && (
          <SharingBody agent={agent} onClose={() => onOpenChange(false)} />
        )}
      </DialogContent>
    </Dialog>
  )
}

function SharingBody({
  agent,
  onClose,
}: {
  agent: AgentInfo
  onClose: () => void
}) {
  const { t, i18n } = useTranslation()
  const { teams, teamsLoading, sharing, loading } = useAgentSharing(agent.id)
  const providers = useAvailableProviders()
  const toggleTeam = useToggleTeamShare(agent.id)
  const setRelay = useSetRelaySharing(agent.id)
  const mine = agent.origin.kind === 'local'

  const card = sharing.shared?.card ?? ownCard(providers.data ?? [], agent.id)
  const capabilities = card?.capabilities ?? []
  const owner = card?.owner.githubHandle

  const goToTeams = () => {
    onClose()
    useNavigationStore.getState().navigate({
      kind: 'network',
      tab: 'people',
      team: MANAGE_TEAMS,
    })
  }

  return (
    <div className="flex flex-col gap-5">
      <section className="flex flex-col gap-2 rounded-md border p-3">
        <h3 className="text-xs font-medium text-muted-foreground uppercase">
          {t('agentSharing.whatOthersSee')}
        </h3>
        <p className="text-sm">
          <span className="font-medium">{agent.name}</span>
          <span className="text-muted-foreground">
            {' · '}
            {owner ? `@${owner}` : t('shared.unknownOwner')}
          </span>
        </p>
        {agent.description && (
          <p className="text-sm text-muted-foreground">{agent.description}</p>
        )}
        {capabilities.length > 0 ? (
          <div className="flex flex-wrap gap-1">
            {capabilities.map(capability => (
              <Badge key={capability.name} variant="secondary">
                {capability.name}
              </Badge>
            ))}
          </div>
        ) : (
          !providers.isLoading && (
            <p className="text-xs text-warning">
              {t('agentSharing.noCapabilities')}
            </p>
          )
        )}
        {sharing.shared && (
          <p className="text-xs text-muted-foreground">
            {t('shared.byMe.calls', { n: sharing.shared.callCount })}
            {' · '}
            {sharing.shared.lastCalledAt
              ? t('shared.byMe.lastCalled', {
                  time: formatRelativeTime(
                    sharing.shared.lastCalledAt,
                    i18n.language
                  ),
                })
              : t('shared.byMe.neverCalled')}
          </p>
        )}
      </section>

      {!mine && (
        <p className="text-sm text-muted-foreground">
          {t('agentSharing.notYours')}
        </p>
      )}

      <section className="flex flex-col gap-2">
        <h3 className="flex items-center gap-2 text-sm font-medium">
          <Users aria-hidden className="size-4 text-muted-foreground" />
          {t('agentSharing.teams')}
        </h3>
        <p className="text-xs text-muted-foreground">
          {t('teams.shareDialog.description')}
        </p>
        {teamsLoading || loading ? (
          <Skeleton className="h-8 w-full" />
        ) : teams.length === 0 ? (
          <div className="flex items-center justify-between gap-2 text-sm text-muted-foreground">
            {t('agentSharing.noTeams')}
            <Button size="sm" variant="outline" onClick={goToTeams}>
              {t('shared.goToTeams')}
            </Button>
          </div>
        ) : (
          <ul className="flex flex-col gap-1">
            {teams.map(team => {
              const id = `share-team-${team.id}`
              return (
                <li
                  key={team.id}
                  className="flex items-center justify-between gap-2 rounded-md px-1 py-1"
                >
                  <Label htmlFor={id} className="font-normal">
                    {team.name}
                  </Label>
                  <Switch
                    id={id}
                    checked={sharing.teamIds.includes(team.id)}
                    disabled={!mine || toggleTeam.isPending}
                    onCheckedChange={share =>
                      toggleTeam.mutate(
                        { teamId: team.id, share },
                        {
                          onError: error =>
                            toast.error(
                              t(
                                share
                                  ? 'teams.error.share'
                                  : 'teams.error.unshare'
                              ),
                              { description: errorMessage(error) }
                            ),
                        }
                      )
                    }
                  />
                </li>
              )
            })}
          </ul>
        )}
      </section>

      <section className="flex flex-col gap-2">
        <div className="flex items-center justify-between gap-2">
          <Label
            htmlFor="share-relay"
            className="flex items-center gap-2 text-sm font-medium"
          >
            <RadioTower aria-hidden className="size-4 text-muted-foreground" />
            {t('agentMenu.relay')}
          </Label>
          <Switch
            id="share-relay"
            checked={sharing.relay}
            disabled={!mine || loading || setRelay.isPending}
            onCheckedChange={on =>
              setRelay.mutate(on, {
                onError: error =>
                  toast.error(t('agentMenu.relayError'), {
                    description: errorMessage(error),
                  }),
              })
            }
          />
        </div>
        <p className="text-xs text-muted-foreground">
          {t('agentSharing.relayDescription')}
        </p>
      </section>
    </div>
  )
}
