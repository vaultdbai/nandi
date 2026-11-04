import { useTranslation } from 'react-i18next'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import {
  useNavigationStore,
  type AutomationsTab,
  type NetworkTab,
  type View,
} from '@/store/navigation-store'
import {
  HowNetworkingWorks,
  NetworkStatusPill,
} from '@/components/network/NetworkHelp'
import { useRelay } from '@/hooks/useRelay'
import { LibraryPageView } from '@/components/library/LibraryView'
import { ConnectionView } from './ConnectionView'
import { NetworkView } from './NetworkView'
import { SharingView } from './SharingView'
import { TriggersListView } from './TriggersListView'
import { WorkflowsListView } from './WorkflowsListView'

const navigate = (view: View) => useNavigationStore.getState().navigate(view)

interface PageTab<T extends string> {
  id: T
  label: string
}

/**
 * A sidebar page: a row of tabs over the view of the active one. Until
 * tasks 8–13 merge them, each tab hosts today's view unchanged (Home is
 * its own page, `HomeView`).
 */
function TabbedPage<T extends string>({
  label,
  tabs,
  active,
  onSelect,
  leading,
  trailing,
  children,
}: {
  label: string
  tabs: PageTab<T>[]
  active: T
  onSelect: (tab: T) => void
  leading?: React.ReactNode
  trailing?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b px-4 py-2">
        {leading}
        <Tabs value={active} onValueChange={value => onSelect(value as T)}>
          <TabsList aria-label={label}>
            {tabs.map(tab => (
              <TabsTrigger key={tab.id} value={tab.id}>
                {tab.label}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        {trailing && <div className="ms-auto">{trailing}</div>}
      </div>
      <div className="min-h-0 flex-1">{children}</div>
    </div>
  )
}

export function AutomationsPage({
  view,
}: {
  view: Extract<View, { kind: 'automations' }>
}) {
  const { t } = useTranslation()
  const tab: AutomationsTab = view.tab ?? 'workflows'

  return (
    <TabbedPage
      label={t('nav.automations')}
      tabs={[
        { id: 'workflows', label: t('nav.workflows') },
        { id: 'triggers', label: t('nav.triggers') },
      ]}
      active={tab}
      onSelect={next => navigate({ kind: 'automations', tab: next })}
    >
      {tab === 'workflows' ? <WorkflowsListView /> : <TriggersListView />}
    </TabbedPage>
  )
}

export function LibraryPage({
  view,
}: {
  view: Extract<View, { kind: 'library' }>
}) {
  return <LibraryPageView view={view} />
}

/**
 * Network: People · Sharing · Connection, with the connection in words
 * (opens Connection) and "How networking works" on every tab. Without a
 * saved relay, Connection is where it starts.
 */
export function NetworkPage({
  view,
}: {
  view: Extract<View, { kind: 'network' }>
}) {
  const { t } = useTranslation()
  const relay = useRelay()
  const fallback: NetworkTab =
    relay.status && !relay.status.relayUrl ? 'connection' : 'people'
  const tab: NetworkTab = view.tab ?? fallback

  return (
    <TabbedPage
      label={t('nav.network')}
      tabs={[
        { id: 'people', label: t('network.tab.people') },
        { id: 'sharing', label: t('network.tab.sharing') },
        { id: 'connection', label: t('network.tab.connection') },
      ]}
      active={tab}
      onSelect={next => navigate({ kind: 'network', tab: next })}
      trailing={
        <div className="flex items-center gap-1">
          <NetworkStatusPill />
          <HowNetworkingWorks />
        </div>
      }
    >
      {tab === 'people' && <NetworkView team={view.team} />}
      {tab === 'sharing' && (
        <SharingView
          tab={view.sharing ?? 'offer'}
          callers={view.callers ?? 'list'}
        />
      )}
      {tab === 'connection' && <ConnectionView />}
    </TabbedPage>
  )
}
