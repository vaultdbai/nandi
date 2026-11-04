import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  ArrowLeft,
  Copy,
  EyeOff,
  RefreshCw,
  Search,
  Share2,
} from 'lucide-react'
import { toast } from 'sonner'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { MemberCardCompact } from '@/components/network/MemberCardCompact'
import { withoutToken } from '@/components/onboarding/identity-model'
import { RelayConnectNotice } from '@/components/network/RelayConnectNotice'
import { useGatewayStatus, useMakeGatewayReachable } from '@/hooks/useGateway'
import {
  filterMembers,
  searchMembers,
  useNetworkSnapshot,
  useRefreshNetwork,
  type MemberFilter,
} from '@/hooks/useNetwork'
import { useCopyInvite, useRelay } from '@/hooks/useRelay'
import { useTeamRoster } from '@/hooks/useTeamRoster'
import { useTeams } from '@/hooks/useTeams'
import { errorMessage } from '@/hooks/useDelegation'
import type { NetworkMember, Team } from '@/lib/bindings'
import { toastPersonError } from '@/lib/toast-error'
import {
  MANAGE_TEAMS,
  useNavigationStore,
  type View,
} from '@/store/navigation-store'
import { ShareAgentDialog, TeamView } from './TeamView'

const navigate = (view: View) => useNavigationStore.getState().navigate(view)

/**
 * Network › People: everyone on the relay by their card, filtered by team,
 * presence or a search. "Manage teams…" (the team filter) opens Teams here.
 */
export function NetworkView({ team }: { team?: string }) {
  const { t } = useTranslation()
  const snapshot = useNetworkSnapshot()
  const refresh = useRefreshNetwork()
  const teams = useTeams()
  const relay = useRelay().status
  const [filter, setFilter] = useState<MemberFilter>('all')
  const [query, setQuery] = useState('')
  const gateway = useGatewayStatus()
  const makeReachable = useMakeGatewayReachable()
  const connected = relay?.connected === true
  // Others only see this Nandi with the gateway on and listed
  const hidden =
    gateway.data !== undefined &&
    !(gateway.data.enabled && gateway.data.discoverable)

  const setTeam = (next: string | undefined) =>
    navigate({ kind: 'network', tab: 'people', team: next })

  if (team === MANAGE_TEAMS) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        <div className="shrink-0 border-b px-4 py-2">
          <Button size="sm" variant="ghost" onClick={() => setTeam(undefined)}>
            <ArrowLeft className="rtl:rotate-180" />
            {t('network.people.back')}
          </Button>
        </div>
        <div className="min-h-0 flex-1">
          <TeamView />
        </div>
      </div>
    )
  }

  const teamList = teams.data ?? []
  const chosenTeam = teamList.find(item => item.id === team)
  const members = snapshot.data?.members ?? []

  const onRefresh = () =>
    refresh.mutate(undefined, {
      onError: error =>
        toastPersonError(error, failure =>
          toast.error(
            t('network.refreshFailed', { error: errorMessage(failure) })
          )
        ),
    })

  const onMakeReachable = () =>
    makeReachable.mutate(undefined, {
      onSuccess: status => {
        if (status.registered) {
          toast.success(t('network.reachable.done'))
        } else {
          toast.warning(t('network.reachable.pending'))
        }
        refresh.mutate()
      },
      onError: error =>
        toast.error(t('network.reachable.failed', { error: error.message })),
    })

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-3 border-b px-4 py-2">
        <div className="relative min-w-48 max-w-md flex-1">
          <Search className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            value={query}
            onChange={event => setQuery(event.target.value)}
            placeholder={t('network.people.search')}
            aria-label={t('network.people.search')}
            className="ps-8"
          />
        </div>
        <NativeSelect
          aria-label={t('network.people.team')}
          value={chosenTeam?.id ?? ''}
          onChange={event => setTeam(event.target.value || undefined)}
        >
          <NativeSelectOption value="">
            {t('network.people.allTeams')}
          </NativeSelectOption>
          {teamList.map(item => (
            <NativeSelectOption key={item.id} value={item.id}>
              {item.name}
            </NativeSelectOption>
          ))}
          <NativeSelectOption value={MANAGE_TEAMS}>
            {t('network.people.manageTeams')}
          </NativeSelectOption>
        </NativeSelect>
        <ToggleGroup
          type="single"
          variant="outline"
          size="sm"
          value={filter}
          onValueChange={value => {
            if (value) setFilter(value as MemberFilter)
          }}
          aria-label={t('network.filter.label')}
        >
          <ToggleGroupItem value="all">
            {t('network.filter.all')}
          </ToggleGroupItem>
          <ToggleGroupItem value="online">
            {t('network.filter.online')}
          </ToggleGroupItem>
          <ToggleGroupItem value="recent">
            {t('network.filter.recent')}
          </ToggleGroupItem>
        </ToggleGroup>
        <Button
          size="sm"
          variant="outline"
          className="ms-auto"
          disabled={refresh.isPending}
          onClick={onRefresh}
        >
          {refresh.isPending ? <Spinner /> : <RefreshCw />}
          {t('network.refresh')}
        </Button>
      </div>
      {chosenTeam && <TeamStrip team={chosenTeam} />}
      {connected && hidden && (
        <div className="border-b px-4 py-3">
          <Alert data-testid="network-not-reachable">
            <EyeOff />
            <AlertTitle>{t('network.reachable.title')}</AlertTitle>
            <AlertDescription className="flex flex-wrap items-center justify-between gap-3">
              <span>{t('network.reachable.description')}</span>
              <Button
                size="sm"
                disabled={makeReachable.isPending}
                onClick={onMakeReachable}
              >
                {makeReachable.isPending && <Spinner />}
                {t('network.reachable.action')}
              </Button>
            </AlertDescription>
          </Alert>
        </div>
      )}
      <ScrollArea className="min-h-0 flex-1">
        {chosenTeam ? (
          <TeamMembers
            teamId={chosenTeam.id}
            members={members}
            filter={filter}
            query={query}
          />
        ) : (
          <MemberGrid
            loading={snapshot.isPending}
            error={snapshot.error}
            members={members}
            visible={searchMembers(filterMembers(members, filter), query)}
            connected={connected}
            // Invites carry the address, never this Nandi's token
            relayUrl={relay?.relayUrl ? withoutToken(relay.relayUrl) : null}
          />
        )}
      </ScrollArea>
    </div>
  )
}

const openProfile = (instanceId: string) =>
  navigate({ kind: 'member-profile', instanceId })

function MemberGrid({
  loading,
  error,
  members,
  visible,
  connected,
  relayUrl,
}: {
  loading: boolean
  error: Error | null
  members: NetworkMember[]
  visible: NetworkMember[]
  connected: boolean
  relayUrl: string | null
}) {
  const { t } = useTranslation()
  const copyInvite = useCopyInvite()
  const span = 'md:col-span-2 xl:col-span-3'
  return (
    <div className="mx-auto grid w-full max-w-5xl grid-cols-1 gap-4 p-6 md:grid-cols-2 xl:grid-cols-3">
      {error && (
        <p className={`text-sm text-destructive ${span}`}>
          {t('network.loadError', { error: error.message })}
        </p>
      )}
      {loading && [0, 1, 2].map(i => <Skeleton key={i} className="h-44" />)}
      {!loading && !connected && members.length === 0 && (
        <RelayConnectNotice className={span} />
      )}
      {!loading && !connected && members.length > 0 && (
        <p
          className={`text-sm text-muted-foreground ${span}`}
          data-testid="network-last-seen"
        >
          {t('network.people.lastSeenNote')}
        </p>
      )}
      {!loading && connected && members.length === 0 && (
        <Empty className={span}>
          <EmptyHeader>
            <EmptyTitle>{t('network.people.alone')}</EmptyTitle>
            <EmptyDescription>{t('network.people.aloneHint')}</EmptyDescription>
          </EmptyHeader>
          {relayUrl && (
            <EmptyContent>
              <Button variant="outline" onClick={() => void copyInvite()}>
                <Copy />
                {t('network.invite.copy')}
              </Button>
            </EmptyContent>
          )}
        </Empty>
      )}
      {members.length > 0 && visible.length === 0 && (
        <p className={`text-sm text-muted-foreground ${span}`}>
          {t('network.noMatches')}
        </p>
      )}
      {visible.map(member => (
        <MemberCardCompact
          key={member.instanceId}
          member={member}
          onOpen={openProfile}
        />
      ))}
    </div>
  )
}

/** The cards of the team's roster that the directory knows */
function TeamMembers({
  teamId,
  members,
  filter,
  query,
}: {
  teamId: string
  members: NetworkMember[]
  filter: MemberFilter
  query: string
}) {
  const { t } = useTranslation()
  const roster = useTeamRoster(teamId)
  const inTeam = new Set(
    (roster.data?.members ?? []).flatMap(m =>
      m.instanceId ? [m.instanceId] : []
    )
  )
  const ofTeam = members.filter(member => inTeam.has(member.instanceId))
  const visible = searchMembers(filterMembers(ofTeam, filter), query)
  if (roster.isPending) {
    return (
      <div className="mx-auto w-full max-w-5xl p-6">
        <Skeleton className="h-44" />
      </div>
    )
  }
  return (
    <div className="mx-auto grid w-full max-w-5xl grid-cols-1 gap-4 p-6 md:grid-cols-2 xl:grid-cols-3">
      {visible.length === 0 && (
        <p className="text-sm text-muted-foreground md:col-span-2 xl:col-span-3">
          {ofTeam.length === 0
            ? t('network.people.teamNobodyOnline')
            : t('network.noMatches')}
        </p>
      )}
      {visible.map(member => (
        <MemberCardCompact
          key={member.instanceId}
          member={member}
          onOpen={openProfile}
        />
      ))}
    </div>
  )
}

/** The chosen team: who's in it, its registry, and Share an agent */
function TeamStrip({ team }: { team: Team }) {
  const { t } = useTranslation()
  const [sharing, setSharing] = useState(false)
  return (
    <div
      className="flex flex-wrap items-center justify-between gap-3 border-b px-4 py-2"
      data-testid="team-strip"
    >
      <div className="min-w-0">
        <p className="truncate text-sm font-medium">{team.name}</p>
        <p className="text-xs text-muted-foreground">
          {t('teams.memberCount', { count: team.members.length })}
        </p>
      </div>
      <div className="flex flex-wrap gap-2">
        <Button size="sm" onClick={() => setSharing(true)}>
          <Share2 />
          {t('network.people.shareWithTeam')}
        </Button>
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            navigate({ kind: 'network', tab: 'people', team: MANAGE_TEAMS })
          }
        >
          {t('network.people.openTeam')}
        </Button>
      </div>
      <ShareAgentDialog
        teamId={team.id}
        teamName={team.name}
        open={sharing}
        onOpenChange={setSharing}
      />
    </div>
  )
}
