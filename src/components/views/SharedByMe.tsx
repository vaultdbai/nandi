import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Search, Share2, Users } from 'lucide-react'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Switch } from '@/components/ui/switch'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { OwnerGroup } from '@/components/network/OwnerGroup'
import {
  filterSharedAgents,
  groupByOwner,
  recentlyUsed,
  useSharedAgents,
  useStopSharing,
  type StopSharing,
} from '@/hooks/useSharedAgents'
import { useRelay } from '@/hooks/useRelay'
import { useTeams } from '@/hooks/useTeams'
import type { SharedAgent } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'
import { MANAGE_TEAMS, useNavigationStore } from '@/store/navigation-store'

/**
 * Sharing › Shared: my shared agents (grouped by owner, like Shared with
 * me), where each is shared, call counts, and Stop sharing (asks first).
 * The trust graph lives in Who may call you.
 */
export function SharedByMeView() {
  return <MySharedAgents />
}

function MySharedAgents() {
  const { t } = useTranslation()
  const { byMe } = useSharedAgents()
  // "(relay)" wording only once a relay URL is set
  const relayConfigured = Boolean(useRelay().status?.relayUrl)
  const [search, setSearch] = useState('')
  const [usage, setUsage] = useState<'all' | 'recent'>('all')
  const agents = byMe.data ?? []
  const matching = filterSharedAgents(agents, { source: 'all', search })
  const visible = usage === 'recent' ? recentlyUsed(matching) : matching

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-3 border-b px-4 py-2">
        <div className="relative min-w-48 max-w-md flex-1">
          <Search className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={search}
            onChange={event => setSearch(event.target.value)}
            placeholder={t('shared.search')}
            aria-label={t('shared.search')}
            className="ps-8"
          />
        </div>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          value={usage}
          onValueChange={value => {
            if (value) setUsage(value as 'all' | 'recent')
          }}
          aria-label={t('shared.filter.label')}
        >
          <ToggleGroupItem value="all">
            {t('shared.filter.all')}
          </ToggleGroupItem>
          <ToggleGroupItem value="recent">
            {t('network.filter.recentlyUsed')}
          </ToggleGroupItem>
        </ToggleGroup>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto grid w-full max-w-5xl grid-cols-1 gap-4 p-6 md:grid-cols-2 xl:grid-cols-3">
          {byMe.error && (
            <p className="text-sm text-destructive md:col-span-2 xl:col-span-3">
              {t('shared.loadError', { error: byMe.error.message })}
            </p>
          )}
          {byMe.isPending &&
            [0, 1, 2].map(i => <Skeleton key={i} className="h-44" />)}
          {!byMe.isPending && !byMe.error && agents.length === 0 && (
            <Empty className="md:col-span-2 xl:col-span-3">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Share2 />
                </EmptyMedia>
                <EmptyTitle>{t('shared.byMe.emptyTitle')}</EmptyTitle>
                <EmptyDescription>
                  {t('shared.byMe.emptyDescription')}
                </EmptyDescription>
              </EmptyHeader>
              <EmptyContent>
                <Button
                  variant="outline"
                  onClick={() =>
                    useNavigationStore.getState().navigate({
                      kind: 'network',
                      tab: 'people',
                      team: MANAGE_TEAMS,
                    })
                  }
                >
                  <Users />
                  {t('shared.goToTeams')}
                </Button>
              </EmptyContent>
            </Empty>
          )}
          {!byMe.isPending &&
            !byMe.error &&
            agents.length > 0 &&
            visible.length === 0 && (
              <p className="text-sm text-muted-foreground md:col-span-2 xl:col-span-3">
                {t('shared.noMatches')}
              </p>
            )}
          {groupByOwner(visible).map(group => (
            // My own agents: no profile link or trust controls
            <OwnerGroup
              key={group.key}
              handle={group.handle}
              displayName={group.displayName}
              count={group.agents.length}
            >
              {group.agents.map(agent => (
                <MySharedAgentCard
                  key={agent.card.id}
                  agent={agent}
                  relayConfigured={relayConfigured}
                />
              ))}
            </OwnerGroup>
          ))}
        </div>
      </ScrollArea>
    </div>
  )
}

function MySharedAgentCard({
  agent,
  relayConfigured,
}: {
  agent: SharedAgent
  relayConfigured: boolean
}) {
  const { t, i18n } = useTranslation()
  const stop = useStopSharing()
  const teams = useTeams()
  const teamName = (teamId: string) =>
    teams.data?.find(team => team.id === teamId)?.name ?? teamId
  const { card } = agent
  const agentId = card.id.startsWith(`${card.instanceId}/`)
    ? card.id.slice(card.instanceId.length + 1)
    : card.id

  const stopSharing = (target: StopSharing) => {
    stop.mutate(target, {
      onSuccess: () => toast.success(t('shared.byMe.stopped')),
      onError: error =>
        toast.error(t('shared.byMe.stopFailed', { error: error.message })),
    })
  }

  return (
    <Card data-testid={`my-shared-agent-${agentId}`} className="gap-3">
      <CardHeader>
        <CardTitle className="truncate">{card.name}</CardTitle>
        <CardDescription>
          {t('shared.byMe.calls', { n: agent.callCount })}
          {' · '}
          {agent.lastCalledAt === null
            ? t('shared.byMe.neverCalled')
            : t('shared.byMe.lastCalled', {
                time: formatRelativeTime(agent.lastCalledAt, i18n.language),
              })}
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <div className="flex flex-wrap gap-1">
          {card.capabilities.map(capability => (
            <Badge key={capability.name} variant="secondary">
              {capability.name}
            </Badge>
          ))}
        </div>
        <div className="flex flex-col gap-2">
          {agent.sources.includes('relay') && (
            <SharingToggle
              id={`share-relay-${agentId}`}
              label={
                relayConfigured
                  ? t('shared.byMe.acceptsDelegationRelay')
                  : t('shared.byMe.acceptsDelegation')
              }
              disabled={stop.isPending}
              onStop={() => stopSharing({ kind: 'relay', agentId })}
            />
          )}
          {agent.teamIds.map(teamId => (
            <SharingToggle
              key={teamId}
              id={`share-team-${agentId}-${teamId}`}
              label={t('shared.byMe.sharedToTeam', { team: teamName(teamId) })}
              disabled={stop.isPending}
              onStop={() => stopSharing({ kind: 'team', teamId, agentId })}
            />
          ))}
        </div>
      </CardContent>
    </Card>
  )
}

/** On while shared; turning it off asks, then stops that way of sharing */
function SharingToggle({
  id,
  label,
  disabled,
  onStop,
}: {
  id: string
  label: string
  disabled: boolean
  onStop: () => void
}) {
  const { t } = useTranslation()
  const [asking, setAsking] = useState(false)
  return (
    <div className="flex items-center justify-between gap-2">
      <Label htmlFor={id} className="text-sm font-normal">
        {label}
      </Label>
      <Switch
        id={id}
        checked
        disabled={disabled}
        onCheckedChange={checked => {
          if (!checked) setAsking(true)
        }}
      />
      <AlertDialog open={asking} onOpenChange={setAsking}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('shared.byMe.stopTitle')}</AlertDialogTitle>
            <AlertDialogDescription>
              {t('shared.byMe.stopBody')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('network.cancel')}</AlertDialogCancel>
            <AlertDialogAction onClick={onStop}>
              {t('shared.byMe.stopConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
