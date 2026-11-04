import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  Compass,
  Crown,
  Handshake,
  Search,
  Server,
  Sparkles,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { CapabilityCard } from '@/components/capability/CapabilityCard'
import {
  allTags,
  CAPABILITY_SECTIONS,
  EMPTY_FILTERS,
  filterListings,
  groupBySection,
  type CapabilityFilters,
  type CapabilitySection,
  type OwnershipFilter,
} from '@/components/capability/capability-model'
import {
  useCapabilityEvents,
  useCapabilityListings,
} from '@/hooks/useCapabilities'
import type { Visibility } from '@/lib/bindings'
import { ViewHeader } from './ViewHeader'
import { ErrorMessage } from '@/components/ui/error-message'
import { plainError } from '@/lib/toast-error'

const SECTION_ICONS: Record<CapabilitySection, React.ReactNode> = {
  owned: <Crown />,
  provided: <Server />,
  used: <Handshake />,
  discover: <Compass />,
}

const VISIBILITIES: Visibility[] = ['private', 'team', 'network']

/**
 * Capabilities you own, provide, use and can discover, with search and
 * filters; each a grid of `CapabilityCard`s with its own empty state.
 * Lives under Network › Sharing (what you offer) since task 11; tools are
 * in the Library.
 */
export function CapabilitiesView() {
  const { t } = useTranslation()
  useCapabilityEvents()
  const listings = useCapabilityListings()
  const [filters, setFilters] = useState<CapabilityFilters>(EMPTY_FILTERS)

  const all = listings.data ?? []
  const groups = groupBySection(filterListings(all, filters))
  const tags = allTags(all)
  const filtering =
    filters.query.trim() !== '' ||
    filters.visibility !== 'all' ||
    filters.tag !== null ||
    filters.ownership !== 'all'

  const update = (patch: Partial<CapabilityFilters>) =>
    setFilters(current => ({ ...current, ...patch }))

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        title={t('capabilities.title')}
        subtitle={t('capabilities.subtitle')}
        leading={<Sparkles className="size-4 text-muted-foreground" />}
      />
      <div className="flex flex-wrap items-center gap-2 border-b px-4 py-2">
        <div className="relative min-w-48 flex-1">
          <Search
            className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden
          />
          <Input
            type="search"
            value={filters.query}
            onChange={event => update({ query: event.target.value })}
            placeholder={t('capabilities.search')}
            aria-label={t('capabilities.search')}
            className="ps-8"
          />
        </div>
        <NativeSelect
          value={filters.visibility}
          aria-label={t('capabilities.filter.visibility')}
          onChange={event =>
            update({
              visibility: event.target.value as CapabilityFilters['visibility'],
            })
          }
        >
          <NativeSelectOption value="all">
            {t('capabilities.filter.anyVisibility')}
          </NativeSelectOption>
          {VISIBILITIES.map(visibility => (
            <NativeSelectOption key={visibility} value={visibility}>
              {t(`capabilities.visibility.${visibility}`)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <NativeSelect
          value={filters.tag ?? ''}
          aria-label={t('capabilities.filter.tag')}
          disabled={tags.length === 0}
          onChange={event => update({ tag: event.target.value || null })}
        >
          <NativeSelectOption value="">
            {t('capabilities.filter.anyTag')}
          </NativeSelectOption>
          {tags.map(tag => (
            <NativeSelectOption key={tag} value={tag}>
              {tag}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        {/* A select, so the labels can't run into each other */}
        <NativeSelect
          value={filters.ownership}
          aria-label={t('capabilities.filter.ownership')}
          onChange={event =>
            update({ ownership: event.target.value as OwnershipFilter })
          }
        >
          <NativeSelectOption value="all">
            {t('capabilities.filter.all')}
          </NativeSelectOption>
          <NativeSelectOption value="mine">
            {t('capabilities.filter.mine')}
          </NativeSelectOption>
          <NativeSelectOption value="others">
            {t('capabilities.filter.others')}
          </NativeSelectOption>
        </NativeSelect>
        {filtering && (
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setFilters(EMPTY_FILTERS)}
          >
            {t('capabilities.filter.clear')}
          </Button>
        )}
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto flex w-full max-w-5xl flex-col gap-8 p-6">
          {listings.error && (
            <ErrorMessage
              error={listings.error}
              message={t('capabilities.loadError', {
                error: plainError(listings.error),
              })}
            />
          )}
          {CAPABILITY_SECTIONS.map(section => (
            <section
              key={section}
              className="flex flex-col gap-3"
              aria-labelledby={`capabilities-section-${section}`}
              data-testid={`section-${section}`}
            >
              <h2
                id={`capabilities-section-${section}`}
                className="flex items-center gap-2 text-sm font-semibold [&_svg]:size-4 [&_svg]:text-muted-foreground"
              >
                {SECTION_ICONS[section]}
                {t(`capabilities.section.${section}.title`)}
                {listings.data && (
                  <span className="text-xs font-normal text-muted-foreground tabular-nums">
                    {groups[section].length}
                  </span>
                )}
              </h2>
              {listings.isPending ? (
                <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
                  <Skeleton className="h-36" />
                  <Skeleton className="h-36" />
                </div>
              ) : groups[section].length === 0 ? (
                <Empty className="border py-8">
                  <EmptyHeader>
                    <EmptyMedia variant="icon">
                      {SECTION_ICONS[section]}
                    </EmptyMedia>
                    <EmptyTitle>
                      {filtering
                        ? t('capabilities.section.noMatch')
                        : t(`capabilities.section.${section}.emptyTitle`)}
                    </EmptyTitle>
                    {!filtering && (
                      <EmptyDescription>
                        {t(`capabilities.section.${section}.emptyDescription`)}
                      </EmptyDescription>
                    )}
                  </EmptyHeader>
                </Empty>
              ) : (
                <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
                  {groups[section].map(listing => (
                    <CapabilityCard
                      key={listing.definition.id}
                      listing={listing}
                    />
                  ))}
                </div>
              )}
            </section>
          ))}
        </div>
      </ScrollArea>
    </div>
  )
}
