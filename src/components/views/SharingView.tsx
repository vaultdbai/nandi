/**
 * Network › Sharing: trust requests on top, then What you offer, Who may
 * call you (list or graph), Incoming requests, Shared, and Capabilities
 * (network.md §4.4).
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Inbox } from 'lucide-react'
import { toast } from 'sonner'
import { TrustGraph } from '@/components/agent-console/TrustGraph'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useMemberNames } from '@/components/agents/task-utils'
import { GatewayCallersTable } from '@/components/gateway/GatewayCallersTable'
import { Term } from '@/components/network/NetworkHelp'
import { OfferView } from '@/components/network/OfferView'
import { RelayConnectNotice } from '@/components/network/RelayConnectNotice'
import { TrustRequestsStrip } from '@/components/network/TrustRequestsStrip'
import { rawError } from '@/lib/friendly-error'
import {
  incomingFilterOf,
  useGatewayCallers,
  useGatewayEvents,
  useGatewayManifest,
  useGatewayStatus,
  useIncomingTasks,
  useRemoveCallerPolicy,
  useSetCallerPolicy,
  type IncomingFilter,
} from '@/hooks/useGateway'
import { useNetworkSnapshot } from '@/hooks/useNetwork'
import { useRelayConnected } from '@/hooks/useRelay'
import type { TaskRecord } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'
import {
  useNavigationStore,
  type CallersView,
  type SharingTab,
  type View,
} from '@/store/navigation-store'
import { CapabilitiesView } from './CapabilitiesView'
import { SharedByMeView } from './SharedByMe'

const navigate = (view: View) => useNavigationStore.getState().navigate(view)

const SHARING_TABS: SharingTab[] = [
  'offer',
  'callers',
  'incoming',
  'shared',
  'capabilities',
]

export function SharingView({
  tab,
  callers,
}: {
  tab: SharingTab
  callers: CallersView
}) {
  const { t } = useTranslation()
  useGatewayEvents()
  // What you offer is set up here either way; others reach it through the relay
  const relayConnected = useRelayConnected()

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-col gap-3 border-b px-4 py-3">
        {relayConnected === false && <RelayConnectNotice variant="inline" />}
        <TrustRequestsStrip />
        <Tabs
          value={tab}
          onValueChange={next =>
            navigate({
              kind: 'network',
              tab: 'sharing',
              sharing: next as SharingTab,
            })
          }
        >
          <TabsList
            aria-label={t('network.tab.sharing')}
            className="h-auto flex-wrap"
          >
            {SHARING_TABS.map(id => (
              <TabsTrigger key={id} value={id}>
                {t(`network.sharing.${id}`)}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
      </div>
      <div className="min-h-0 flex-1">
        {tab === 'shared' && <SharedByMeView />}
        {tab === 'capabilities' && <CapabilitiesView />}
        {(tab === 'offer' || tab === 'callers' || tab === 'incoming') && (
          <ScrollArea className="h-full">
            <div className="mx-auto w-full max-w-5xl p-6">
              {tab === 'offer' && <OfferView />}
              {tab === 'callers' && <CallersPanel view={callers} />}
              {tab === 'incoming' && <IncomingRequests />}
            </div>
          </ScrollArea>
        )}
      </div>
    </div>
  )
}

/** Who may call you: the rule in one sentence, then the people (or graph) */
function CallersPanel({ view }: { view: CallersView }) {
  const { t } = useTranslation()
  const manifest = useGatewayManifest()
  const callers = useGatewayCallers()
  const setCaller = useSetCallerPolicy()
  const removeCaller = useRemoveCallerPolicy()
  const nameOf = useMemberNames()
  const trusted = manifest.data?.federation.trustedPeers.length ?? 0
  const onError = (error: unknown) => toast.error(rawError(error))

  return (
    <section className="flex flex-col gap-4" aria-labelledby="callers-title">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 id="callers-title" className="text-sm font-semibold">
          {t('network.sharing.callers')}
        </h2>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          value={view}
          onValueChange={value => {
            if (value) {
              navigate({
                kind: 'network',
                tab: 'sharing',
                sharing: 'callers',
                callers: value as CallersView,
              })
            }
          }}
          aria-label={t('network.callers.viewLabel')}
        >
          <ToggleGroupItem value="list">
            {t('network.callers.list')}
          </ToggleGroupItem>
          <ToggleGroupItem value="graph">
            {t('network.callers.graph')}
          </ToggleGroupItem>
        </ToggleGroup>
      </div>
      <p className="text-sm" data-testid="callers-summary">
        <Term term="trusted">
          {t('network.callers.trustedCount', { count: trusted })}
        </Term>{' '}
        {t('network.callers.summaryRest')}{' '}
        <Term term="asksFirst">{t('network.callers.asksFirst')}</Term>
      </p>
      {view === 'graph' ? (
        <TrustGraph />
      ) : (
        <GatewayCallersTable
          callers={callers.data ?? []}
          nameOf={nameOf}
          busy={setCaller.isPending || removeCaller.isPending}
          onSave={policy => setCaller.mutate(policy, { onError })}
          onRemove={caller => removeCaller.mutate(caller, { onError })}
        />
      )}
    </section>
  )
}

const FILTERS: IncomingFilter[] = ['waiting', 'running', 'done', 'refused']

/** Requests other Nandis sent you, refused ones included */
function IncomingRequests() {
  const { t, i18n } = useTranslation()
  const tasks = useIncomingTasks()
  const gateway = useGatewayStatus()
  const snapshot = useNetworkSnapshot()
  const nameOf = useMemberNames()
  const [filter, setFilter] = useState<IncomingFilter | 'all'>('all')
  const all = tasks.data ?? []
  const visible =
    filter === 'all'
      ? all
      : all.filter(task => incomingFilterOf(task.state) === filter)
  const reachable = Boolean(gateway.data?.enabled && gateway.data.discoverable)

  return (
    <section className="flex flex-col gap-4" aria-labelledby="incoming-title">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 id="incoming-title" className="text-sm font-semibold">
          {t('network.sharing.incoming')}
        </h2>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          className="flex-wrap"
          value={filter}
          onValueChange={value => {
            if (value) setFilter(value as IncomingFilter | 'all')
          }}
          aria-label={t('network.incoming.filterLabel')}
        >
          <ToggleGroupItem value="all">
            {t('network.incoming.filter.all')}
          </ToggleGroupItem>
          {FILTERS.map(id => (
            <ToggleGroupItem key={id} value={id}>
              {t(`network.incoming.filter.${id}`)}
            </ToggleGroupItem>
          ))}
        </ToggleGroup>
      </div>
      {tasks.isPending && <Skeleton className="h-32 w-full" />}
      {!tasks.isPending && all.length === 0 && (
        <Empty className="border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Inbox />
            </EmptyMedia>
            <EmptyTitle>{t('network.incoming.empty')}</EmptyTitle>
            <EmptyDescription>
              {t('network.incoming.emptyHint', {
                count: snapshot.data?.members.length ?? 0,
              })}
            </EmptyDescription>
          </EmptyHeader>
          {!reachable && (
            <EmptyContent>
              <Button
                onClick={() => navigate({ kind: 'network', tab: 'connection' })}
              >
                {t('network.incoming.makeReachable')}
              </Button>
            </EmptyContent>
          )}
        </Empty>
      )}
      {all.length > 0 && visible.length === 0 && (
        <p className="text-sm text-muted-foreground">
          {t('network.incoming.noMatches')}
        </p>
      )}
      <ul className="grid gap-2" data-testid="incoming-requests">
        {visible.map(task => (
          <IncomingRow
            key={task.id}
            task={task}
            from={nameOf(task.requesterInstance) ?? t('network.unnamed')}
            when={formatRelativeTime(task.createdAt, i18n.language)}
          />
        ))}
      </ul>
    </section>
  )
}

function IncomingRow({
  task,
  from,
  when,
}: {
  task: TaskRecord
  from: string
  when: string
}) {
  const { t } = useTranslation()
  const filter = incomingFilterOf(task.state)
  const refusedForTrust =
    filter === 'refused' && /trust/i.test(task.error ?? '')
  return (
    <li>
      <button
        type="button"
        data-testid={`incoming-${task.id}`}
        onClick={() => navigate({ kind: 'request', requestId: task.id })}
        className="flex w-full items-center justify-between gap-3 rounded-md border px-3 py-2 text-start text-sm hover:bg-accent/40"
      >
        <div className="min-w-0">
          <div className="truncate">
            {t('network.incoming.row', {
              capability: task.capability,
              agent: task.providerAgent,
            })}
          </div>
          <div className="truncate text-xs text-muted-foreground">
            {t('network.incoming.from', { from, when })}
            {filter === 'refused' &&
              ` · ${t(
                refusedForTrust
                  ? 'network.incoming.refusedTrust'
                  : 'network.incoming.refusedOther'
              )}`}
          </div>
        </div>
        <Badge variant={filter === 'refused' ? 'outline' : 'secondary'}>
          {t(`network.incoming.filter.${filter}`)}
        </Badge>
      </button>
    </li>
  )
}
