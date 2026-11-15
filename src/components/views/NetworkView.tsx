import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Copy, EyeOff, RefreshCw, Search } from 'lucide-react'
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
import { myGroups, peopleInGroup } from '@/components/network/groups-model'
import { withoutToken } from '@/components/onboarding/identity-model'
import { RelayConnectNotice } from '@/components/network/RelayConnectNotice'
import { useGatewayStatus, useMakeGatewayReachable } from '@/hooks/useGateway'
import { useGroup, useGroups } from '@/hooks/useGroups'
import {
  filterMembers,
  searchMembers,
  useNetworkSnapshot,
  useRefreshNetwork,
  type MemberFilter,
} from '@/hooks/useNetwork'
import { useCopyInvite, useRelay } from '@/hooks/useRelay'
import { rawError } from '@/lib/friendly-error'
import type { NetworkMember } from '@/lib/bindings'
import { toastPersonError } from '@/lib/toast-error'
import { useNavigationStore, type View } from '@/store/navigation-store'

const navigate = (view: View) => useNavigationStore.getState().navigate(view)

/** The group filter's "Manage groups…" choice: opens Network › Groups */
const MANAGE_GROUPS = '__manage'

/**
 * Network › People: everyone on the relay by their card, filtered by one of
 * my groups, presence or a search. "Manage groups…" opens Groups.
 */
export function NetworkView({ memberOf }: { memberOf?: string }) {
  const { t } = useTranslation()
  const snapshot = useNetworkSnapshot()
  const refresh = useRefreshNetwork()
  const groups = myGroups(useGroups().data?.groups ?? [])
  const chosenGroup = groups.find(group => group.id === memberOf)
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

  const members = snapshot.data?.members ?? []

  const setGroup = (next: string) =>
    navigate(
      next === MANAGE_GROUPS
        ? { kind: 'network', tab: 'groups' }
        : { kind: 'network', tab: 'people', memberOf: next || undefined }
    )

  const onRefresh = () =>
    refresh.mutate(undefined, {
      onError: error =>
        toastPersonError(error, failure =>
          toast.error(t('network.refreshFailed', { error: rawError(failure) }))
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
          aria-label={t('network.people.group')}
          value={chosenGroup?.id ?? ''}
          onChange={event => setGroup(event.target.value)}
        >
          <NativeSelectOption value="">
            {t('network.people.allGroups')}
          </NativeSelectOption>
          {groups.map(group => (
            <NativeSelectOption key={group.id} value={group.id}>
              {group.name}
            </NativeSelectOption>
          ))}
          <NativeSelectOption value={MANAGE_GROUPS}>
            {t('network.people.manageGroups')}
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
      {connected && relay?.visibility === 'groups' && (
        <p
          className="border-b px-4 py-2 text-sm text-muted-foreground"
          data-testid="network-groups-visibility"
        >
          {t('network.people.groupsVisibility')}
        </p>
      )}
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
        {chosenGroup ? (
          <GroupPeople
            groupId={chosenGroup.id}
            people={members}
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

/** The directory's people in one of my groups, with a way to open the group */
function GroupPeople({
  groupId,
  people,
  filter,
  query,
}: {
  groupId: string
  people: NetworkMember[]
  filter: MemberFilter
  query: string
}) {
  const { t } = useTranslation()
  const group = useGroup(groupId)
  const span = 'md:col-span-2 xl:col-span-3'
  if (group.isPending) {
    return (
      <div className="mx-auto w-full max-w-5xl p-6">
        <Skeleton className="h-44" />
      </div>
    )
  }
  const inGroup = peopleInGroup(people, group.data?.group.members ?? [])
  const visible = searchMembers(filterMembers(inGroup, filter), query)
  return (
    <div className="mx-auto grid w-full max-w-5xl grid-cols-1 gap-4 p-6 md:grid-cols-2 xl:grid-cols-3">
      <div className={`flex justify-end ${span}`}>
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            navigate({ kind: 'network', tab: 'groups', group: groupId })
          }
        >
          {t('network.people.openGroup')}
        </Button>
      </div>
      {visible.length === 0 && (
        <p className={`text-sm text-muted-foreground ${span}`}>
          {inGroup.length === 0
            ? t('network.people.groupNobodyOnline')
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
