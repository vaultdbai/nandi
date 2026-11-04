import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Download, PhoneOutgoing, RadioTower, Users } from 'lucide-react'
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
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { CallDialog, type CallRequest } from '@/components/sharing/CallDialog'
import { CapabilityProvidersList } from '@/components/sharing/CapabilityProvidersList'
import { OwnerGroup } from '@/components/network/OwnerGroup'
import { TrustPopover } from '@/components/network/TrustControls'
import {
  filterSharedAgents,
  groupByOwner,
  providerCounts,
  recentlyUsed,
  useCapabilityProviders,
  useSharedAgents,
  type SourceFilter,
} from '@/hooks/useSharedAgents'
import { useRelay } from '@/hooks/useRelay'
import type { SharedAgent } from '@/lib/bindings'
import { useNavigationStore, type View } from '@/store/navigation-store'

/** A source, or "used in the last 7 days" (any source) */
type AgentFilter = SourceFilter | 'recent'

/**
 * Library › Shared with me: agents people on my network share, grouped by
 * owner, each with Ask. The page's search and Can do filter apply; with a
 * capability picked, a Providers strip shows who else serves it and lets
 * me pin one. "From" is a select so its labels never overlap.
 */
export function SharedWithMeView({
  search = '',
  canDo = null,
}: {
  search?: string
  canDo?: string | null
}) {
  const { t } = useTranslation()
  const { withMe } = useSharedAgents()
  const overview = useCapabilityProviders()
  const relay = useRelay()
  const [source, setSource] = useState<AgentFilter>('all')
  const [calling, setCalling] = useState<CallRequest | null>(null)

  const agents = withMe.data ?? []
  const searched =
    source === 'recent'
      ? recentlyUsed(filterSharedAgents(agents, { source: 'all', search }))
      : filterSharedAgents(agents, { source, search })
  const visible =
    canDo === null
      ? searched
      : searched.filter(agent =>
          agent.card.capabilities.some(
            capability => capability.name.toLowerCase() === canDo.toLowerCase()
          )
        )
  const counts = providerCounts(overview.data)
  const connected = relay.status?.connected ?? false
  const go = (view: View) => useNavigationStore.getState().navigate(view)

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-2 border-b px-4 py-2 text-sm">
        <label className="flex items-center gap-2">
          <span className="text-muted-foreground">
            {t('shared.filter.from')}
          </span>
          <NativeSelect
            value={source}
            onChange={event => setSource(event.target.value as AgentFilter)}
            className="h-8"
          >
            <NativeSelectOption value="all">
              {t('shared.filter.everyone')}
            </NativeSelectOption>
            <NativeSelectOption value="team">
              {t('shared.filter.myTeams')}
            </NativeSelectOption>
            <NativeSelectOption value="relay">
              {t('shared.source.relay')}
            </NativeSelectOption>
            <NativeSelectOption value="recent">
              {t('shared.filter.recent')}
            </NativeSelectOption>
          </NativeSelect>
        </label>
      </div>
      <ScrollArea className="min-h-0 flex-1">
        {canDo !== null && (
          <div className="mx-auto w-full max-w-5xl px-6 pt-6">
            <CapabilityProvidersList search={canDo} onCall={setCalling} />
          </div>
        )}
        <div className="mx-auto grid w-full max-w-5xl grid-cols-1 gap-4 p-6 md:grid-cols-2 xl:grid-cols-3">
          {withMe.error && (
            <p className="text-sm text-destructive md:col-span-2 xl:col-span-3">
              {t('shared.loadError', { error: withMe.error.message })}
            </p>
          )}
          {withMe.isPending &&
            [0, 1, 2].map(i => <Skeleton key={i} className="h-44" />)}
          {!withMe.isPending && !withMe.error && agents.length === 0 && (
            <Empty className="md:col-span-2 xl:col-span-3">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Users />
                </EmptyMedia>
                <EmptyTitle>
                  {connected
                    ? t('shared.withMe.noneTitle')
                    : t('shared.withMe.notConnectedTitle')}
                </EmptyTitle>
                <EmptyDescription>
                  {connected
                    ? t('shared.withMe.noneDescription')
                    : t('shared.withMe.notConnectedDescription')}
                </EmptyDescription>
              </EmptyHeader>
              <EmptyContent>
                {connected ? (
                  <Button
                    onClick={() => go({ kind: 'network', tab: 'people' })}
                  >
                    <Users />
                    {t('shared.findPeople')}
                  </Button>
                ) : (
                  <Button
                    onClick={() => go({ kind: 'network', tab: 'connection' })}
                  >
                    <RadioTower />
                    {t('shared.goToNetwork')}
                  </Button>
                )}
              </EmptyContent>
            </Empty>
          )}
          {!withMe.isPending &&
            !withMe.error &&
            agents.length > 0 &&
            visible.length === 0 && (
              <p className="text-sm text-muted-foreground md:col-span-2 xl:col-span-3">
                {t('shared.noMatches')}
              </p>
            )}
          {groupByOwner(visible).map(group => (
            <OwnerGroup
              key={group.key}
              handle={group.handle}
              displayName={group.displayName}
              count={group.agents.length}
              instanceId={group.instanceId}
              actions={
                group.instanceId && (
                  <TrustPopover instanceId={group.instanceId} />
                )
              }
            >
              {group.agents.map(agent => (
                <SharedAgentCard
                  key={agent.card.id}
                  agent={agent}
                  counts={counts}
                  onCall={() =>
                    setCalling({
                      title: t('shared.callTitle', {
                        name: agent.card.name,
                      }),
                      capabilities: agent.card.capabilities.map(c => c.name),
                      target: agent.card.id,
                    })
                  }
                />
              ))}
            </OwnerGroup>
          ))}
        </div>
      </ScrollArea>
      <CallDialog request={calling} onClose={() => setCalling(null)} />
    </div>
  )
}

function SharedAgentCard({
  agent,
  counts,
  onCall,
}: {
  agent: SharedAgent
  /** Agents serving each capability */
  counts: Map<string, number>
  onCall: () => void
}) {
  const { t } = useTranslation()
  const { card } = agent
  const handle = card.owner.githubHandle
  // A name, with the handle in the tooltip (§4.7)
  const owner = card.owner.displayName || (handle ? `@${handle}` : '')
  return (
    <Card data-testid={`shared-agent-${card.id}`} className="gap-3">
      <CardHeader>
        <CardTitle className="truncate">{card.name}</CardTitle>
        <CardDescription
          className="truncate"
          title={handle ? `@${handle}` : undefined}
        >
          {owner || t('shared.unknownOwner')}
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-2">
        {card.description && (
          <p className="line-clamp-2 text-sm text-muted-foreground">
            {card.description}
          </p>
        )}
        <div className="flex flex-wrap gap-1">
          {card.capabilities.map(capability => {
            const providers = counts.get(capability.name) ?? 0
            return (
              <Badge
                key={capability.name}
                variant="secondary"
                title={
                  providers > 1
                    ? t('shared.capabilities.alsoServed', { count: providers })
                    : undefined
                }
              >
                {capability.name}
                {providers > 1 && (
                  <span className="ms-1 rounded-full bg-primary/15 px-1.5 text-[10px] font-semibold text-primary tabular-nums">
                    {providers}
                  </span>
                )}
              </Badge>
            )
          })}
        </div>
        <div className="flex flex-wrap gap-1">
          {agent.sources.map(source => (
            <Badge key={source} variant="outline">
              {t(`shared.source.${source}`)}
            </Badge>
          ))}
          {agent.teamIds.map(teamId => (
            <Badge key={teamId} variant="outline" className="font-normal">
              {teamId}
            </Badge>
          ))}
        </div>
      </CardContent>
      <CardFooter className="mt-auto gap-2">
        <Button
          size="sm"
          onClick={onCall}
          disabled={card.capabilities.length === 0}
        >
          <PhoneOutgoing />
          {t('shared.call')}
        </Button>
        <Tooltip>
          <TooltipTrigger asChild>
            {/* A disabled button gets no pointer events; the span shows the tooltip */}
            <span tabIndex={0}>
              <Button size="sm" variant="outline" disabled>
                <Download />
                {t('shared.install')}
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent>{t('shared.installUnavailable')}</TooltipContent>
        </Tooltip>
      </CardFooter>
    </Card>
  )
}
