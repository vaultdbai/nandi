import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ArrowRight, PackageOpen, RefreshCw, Search, X } from 'lucide-react'
import { useQueryClient } from '@tanstack/react-query'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { useNow } from '@/components/agents/task-utils'
import { SharedWithMeView } from '@/components/views/SharedWithMe'
import { useConnectors } from '@/hooks/useConnectors'
import {
  marketplaceQueryKeys,
  useMarketplaceAgents,
  useMarketplacePacks,
  useRegistries,
} from '@/hooks/useMarketplace'
import { currentPlatform, runsOn, usePilots } from '@/hooks/usePilots'
import { useSharedAgents } from '@/hooks/useSharedAgents'
import { useTools } from '@/hooks/useTools'
import { isDesktop } from '@/lib/client'
import { formatRelativeTime } from '@/lib/format-time'
import { cn } from '@/lib/utils'
import {
  useNavigationStore,
  type LibraryKindFilter,
  type LibraryTab,
  type View,
} from '@/store/navigation-store'
import { InstallConsentDialog } from './InstallConsentDialog'
import { LocalPackInstall } from './LocalPackInstall'
import { libraryEntries, type LibraryEntry } from './library-entry'
import {
  applyKind,
  canDoOptions,
  filterItems,
  itemsForTab,
  kindCounts,
  LIBRARY_KINDS,
  oldestFetch,
  STALE_SOURCES_MS,
  type LibraryItem,
} from './library-model'
import { LibraryCard } from './LibraryCard'
import { LibraryDrawer } from './LibraryDrawer'

type LibraryView = Extract<View, { kind: 'library' }>

const TABS: LibraryTab[] = ['installed', 'browse', 'shared']

/**
 * Library (task 11): everything an agent can be given (agents, the
 * workflows packs bring, tools, packs) in three tabs, filtered by kind,
 * search and Can do, with one card per item and a drawer for the rest.
 * The view holds the tab, kind, Can do and open item, so Back closes the
 * drawer and filters survive.
 */
export function LibraryPageView({ view }: { view: LibraryView }) {
  const { t, i18n } = useTranslation()
  const queryClient = useQueryClient()
  const tab: LibraryTab = view.tab ?? 'installed'
  const kind: LibraryKindFilter = view.filter ?? 'all'
  const canDo = view.capability ?? null
  // Search is per tab, for the session
  const [queries, setQueries] = useState<Record<LibraryTab, string>>({
    installed: '',
    browse: '',
    shared: '',
  })
  const [installing, setInstalling] = useState<LibraryEntry | null>(null)

  const agents = useMarketplaceAgents()
  const packs = useMarketplacePacks()
  const tools = useTools()
  const connectors = useConnectors()
  const pilots = usePilots()
  const registries = useRegistries()
  const { withMe } = useSharedAgents()

  const platform = currentPlatform()
  const entries = libraryEntries({
    agents: agents.data ?? [],
    packs: packs.data ?? [],
    tools: tools.data ?? [],
    connectors: connectors.data ?? [],
    // A browser on another machine: show the host's pilots as they are
    pilots: pilots.pilots,
    pilotStatuses: pilots.statuses,
    runsHere: pilot => !isDesktop() || runsOn(pilot, platform),
  })
  const byKey = new Map(entries.map(entry => [entry.item.key, entry]))

  const go = (patch: Partial<LibraryView>) =>
    useNavigationStore.getState().navigate({ ...view, ...patch })
  const openDrawer = (item: LibraryItem) => go({ item: item.key })
  const closeDrawer = () =>
    useNavigationStore.getState().replace({ ...view, item: undefined })

  const query = queries[tab]
  const tabItems =
    tab === 'shared'
      ? []
      : itemsForTab(
          tab,
          entries.map(entry => entry.item)
        )
  const searched = filterItems(tabItems, { query, canDo })
  const counts = kindCounts(searched)
  const visible = applyKind(searched, kind)
  const sharedCapabilities = (withMe.data ?? []).flatMap(agent =>
    agent.card.capabilities.map(capability => capability.name)
  )
  const options = canDoOptions(
    entries.map(entry => entry.item),
    sharedCapabilities
  )
  const loading =
    tab !== 'shared' && (agents.isPending || packs.isPending || tools.isPending)
  const sourcesFailed = tab === 'browse' && agents.isError && packs.isError
  const updatedAt = oldestFetch((registries.data ?? []).map(r => r.fetchedAt))
  // Once a minute is plenty for "Updated 5 min ago" turning amber
  const now = useNow(tab === 'browse', 60_000)

  const refresh = () => {
    void queryClient.invalidateQueries({ queryKey: marketplaceQueryKeys.all })
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 flex-wrap items-start gap-3 border-b px-4 py-3">
        <div className="min-w-0 flex-1">
          <h1 className="text-base font-semibold">{t('nav.library')}</h1>
          <p className="text-sm text-muted-foreground">
            {t('library.subtitle')}
          </p>
        </div>
        <Tabs
          value={tab}
          onValueChange={next =>
            go({ tab: next as LibraryTab, filter: undefined, item: undefined })
          }
        >
          <TabsList aria-label={t('nav.library')}>
            {TABS.map(id => (
              <TabsTrigger key={id} value={id} className="whitespace-nowrap">
                {t(`library.tab.${id}`)}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        {tab !== 'shared' && <LocalPackInstall />}
      </header>

      <div className="flex shrink-0 flex-col gap-2 border-b px-4 py-2">
        {tab !== 'shared' && (
          <div
            role="group"
            aria-label={t('library.kind.label')}
            className="flex flex-wrap gap-1.5"
          >
            {(['all', ...LIBRARY_KINDS] as LibraryKindFilter[]).map(id => (
              <Button
                key={id}
                size="sm"
                variant={kind === id ? 'secondary' : 'ghost'}
                aria-pressed={kind === id}
                className="h-7 whitespace-nowrap"
                onClick={() => go({ filter: id === 'all' ? undefined : id })}
              >
                {t(`library.kind.${id}`)}
                <span className="text-muted-foreground tabular-nums">
                  {loading ? '–' : counts[id]}
                </span>
              </Button>
            ))}
          </div>
        )}
        <div className="flex flex-wrap items-center gap-2">
          <div className="relative min-w-48 flex-1">
            <Search
              aria-hidden
              className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
            />
            <Input
              type="search"
              value={query}
              onChange={event =>
                setQueries(current => ({
                  ...current,
                  [tab]: event.target.value,
                }))
              }
              placeholder={t('library.search')}
              aria-label={t('library.search')}
              className="ps-8"
            />
          </div>
          <label className="flex items-center gap-2 text-sm">
            <span className="whitespace-nowrap text-muted-foreground">
              {t('library.canDo.label')}
            </span>
            <NativeSelect
              value={canDo ?? ''}
              onChange={event =>
                go({ capability: event.target.value || undefined })
              }
              className="h-8"
            >
              <NativeSelectOption value="">
                {t('library.canDo.any')}
              </NativeSelectOption>
              {options.map(option => (
                <NativeSelectOption key={option} value={option}>
                  {option}
                </NativeSelectOption>
              ))}
            </NativeSelect>
          </label>
          {tab === 'browse' && (
            <div className="flex items-center gap-2 text-xs">
              {updatedAt !== null && (
                <span
                  className={cn(
                    'text-muted-foreground',
                    now - updatedAt > STALE_SOURCES_MS && 'text-warning'
                  )}
                >
                  {t('library.sources.updated', {
                    time: formatRelativeTime(updatedAt, i18n.language),
                  })}
                </span>
              )}
              <Button
                size="icon"
                variant="ghost"
                className="size-7"
                aria-label={t('library.sources.refresh')}
                onClick={refresh}
              >
                <RefreshCw />
              </Button>
              <Button
                size="sm"
                variant="link"
                className="h-auto p-0"
                onClick={() =>
                  useNavigationStore
                    .getState()
                    .navigate({ kind: 'settings', section: 'sources' })
                }
              >
                {t('library.sources.link')}
                <ArrowRight className="rtl:rotate-180" />
              </Button>
            </div>
          )}
        </div>
        {canDo && (
          <div className="flex">
            <Button
              size="sm"
              variant="outline"
              className="h-7"
              onClick={() => go({ capability: undefined })}
            >
              {t('library.canDo.chip', { name: canDo })}
              <X />
            </Button>
          </div>
        )}
      </div>

      {tab === 'shared' ? (
        <div className="min-h-0 flex-1">
          <SharedWithMeView search={query} canDo={canDo} />
        </div>
      ) : (
        <ScrollArea className="min-h-0 flex-1">
          <div className="mx-auto w-full max-w-7xl p-4">
            {loading ? (
              <Grid>
                {[0, 1, 2, 3, 4, 5].map(i => (
                  <Skeleton key={i} className="h-44 rounded-xl" />
                ))}
              </Grid>
            ) : sourcesFailed ? (
              <EmptyNext
                title={t('library.empty.failedTitle')}
                description={t('library.empty.failedDescription')}
                action={t('library.empty.tryAgain')}
                onAction={refresh}
              />
            ) : visible.length === 0 ? (
              <TabEmpty
                tab={tab}
                kind={kind}
                filtering={query.trim() !== '' || canDo !== null}
                query={query}
                onClear={() => {
                  setQueries(current => ({ ...current, [tab]: '' }))
                  go({ capability: undefined, filter: undefined })
                }}
                onBrowse={filter => go({ tab: 'browse', filter })}
              />
            ) : (
              <Grid>
                {visible.map(item => {
                  const entry = byKey.get(item.key)
                  return entry ? (
                    <LibraryCard
                      key={item.key}
                      entry={entry}
                      onOpenDrawer={openDrawer}
                      onInstall={setInstalling}
                    />
                  ) : null
                })}
              </Grid>
            )}
          </div>
        </ScrollArea>
      )}

      <LibraryDrawer
        entry={view.item ? (byKey.get(view.item) ?? null) : null}
        onClose={closeDrawer}
        onInstall={setInstalling}
        onOpenDrawer={openDrawer}
      />
      <InstallConsentDialog
        entry={installing}
        onOpenChange={open => !open && setInstalling(null)}
      />
    </div>
  )
}

function Grid({ children }: { children: React.ReactNode }) {
  return (
    <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
      {children}
    </div>
  )
}

function EmptyNext({
  title,
  description,
  action,
  onAction,
}: {
  title: string
  description: string
  action: string
  onAction: () => void
}) {
  return (
    <Empty className="py-12">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <PackageOpen />
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription>{description}</EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button onClick={onAction}>{action}</Button>
      </EmptyContent>
    </Empty>
  )
}

/** Every empty state names one next step (§4.8) */
function TabEmpty({
  tab,
  kind,
  filtering,
  query,
  onClear,
  onBrowse,
}: {
  tab: Exclude<LibraryTab, 'shared'>
  kind: LibraryKindFilter
  filtering: boolean
  query: string
  onClear: () => void
  onBrowse: (filter: LibraryKindFilter) => void
}) {
  const { t } = useTranslation()
  if (filtering) {
    return (
      <EmptyNext
        title={
          query.trim()
            ? t('library.empty.noMatchTitle', { query: query.trim() })
            : t('library.empty.noMatchFilterTitle')
        }
        description={t('library.empty.noMatchDescription')}
        action={t('library.empty.clearFilters')}
        onAction={onClear}
      />
    )
  }
  if (tab === 'installed') {
    return kind === 'agents' ? (
      <EmptyNext
        title={t('library.empty.noAgentsTitle')}
        description={t('library.empty.noAgentsDescription')}
        action={t('library.empty.browseAgents')}
        onAction={() => onBrowse('agents')}
      />
    ) : (
      <EmptyNext
        title={t('library.empty.nothingTitle')}
        description={t('library.empty.nothingDescription')}
        action={t('library.empty.browsePacks')}
        onAction={() => onBrowse('packs')}
      />
    )
  }
  return (
    <EmptyNext
      title={t('library.empty.noSourcesTitle')}
      description={t('library.empty.noSourcesDescription')}
      action={t('library.empty.addSource')}
      onAction={() =>
        useNavigationStore
          .getState()
          .navigate({ kind: 'settings', section: 'sources' })
      }
    />
  )
}
