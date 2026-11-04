import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ArrowLeft, Download } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { useNow } from '@/components/agents/task-utils'
import { ViewHeader } from '@/components/views/ViewHeader'
import { useAgentNames } from '@/hooks/useHome'
import { useDelegation } from '@/hooks/useDelegation'
import {
  downloadText,
  exportFilename,
  fetchAllRuns,
  useRuns,
} from '@/hooks/useRuns'
import { formatDuration, formatRelativeTime } from '@/lib/format-time'
import { formatCostUsd, formatTokenCount } from '@/lib/format-usage'
import { useNavigationStore } from '@/store/navigation-store'
import {
  activityFacets,
  activityRows,
  activitySummary,
  activityToCsv,
  filterActivity,
  hasActivityFilters,
  startOfDay,
  type ActivityDirection,
  type ActivityFilters,
  type ActivityRange,
  type ActivitySource,
  type ActivityStatus,
} from './activity-model'
import { SourceChip } from './HomeRows'
import { activityHomeRow } from './home-model'
import { toastError } from '@/lib/toast-error'

const DAY_MS = 24 * 60 * 60 * 1000
/** Rows shown before "Load more" */
const PAGE = 100
/** Runs one query reads (the server's page limit) */
const RUNS_READ = 200

const RANGES: ActivityRange[] = ['today', '7d', '30d', 'all']
const STATUSES: ActivityStatus[] = [
  'running',
  'waiting',
  'done',
  'failed',
  'cancelled',
]
const SOURCES: ActivitySource[] = [
  'you',
  'trigger',
  'schedule',
  'workflow',
  'peer',
]
const DIRECTIONS: ActivityDirection[] = ['local', 'incoming', 'outgoing']

/** Where the runs query starts: day-aligned, so the key stays stable */
function queryStart(range: ActivityRange | undefined, now: number) {
  const today = startOfDay(now)
  switch (range ?? '7d') {
    case 'today':
      return today
    case '7d':
      return today - 7 * DAY_MS
    case '30d':
      return today - 30 * DAY_MS
    case 'all':
      return undefined
  }
}

/**
 * Home › All activity: runs and delegations (both ways) in one list,
 * filtered by date, status, source, direction, agent and member, with
 * Export CSV of exactly the filtered rows. Filters live in the view, so
 * they survive Back.
 */
export function AllActivityView({
  filters = {},
}: {
  filters?: ActivityFilters
}) {
  const { t, i18n } = useTranslation()
  const now = useNow(true, 60_000)
  const [shown, setShown] = useState(PAGE)
  const [exporting, setExporting] = useState(false)
  const agentName = useAgentNames()
  const dateFrom = queryStart(filters.range, now)
  const runs = useRuns(dateFrom === undefined ? {} : { dateFrom }, 0, RUNS_READ)
  const delegation = useDelegation()
  const all = activityRows(runs.data?.rows ?? [], delegation.history, agentName)
  const rows = filterActivity(all, filters, now)
  const facets = activityFacets(all)
  const summary = activitySummary(rows)
  const filtered = hasActivityFilters(filters)

  const setFilters = (next: ActivityFilters) => {
    setShown(PAGE)
    const clean = Object.fromEntries(
      Object.entries(next).filter(
        ([, value]) => value !== undefined && value !== ''
      )
    ) as ActivityFilters
    useNavigationStore
      .getState()
      .replace({ kind: 'home-activity', filters: clean })
  }
  const set = <K extends keyof ActivityFilters>(
    key: K,
    value: ActivityFilters[K] | ''
  ) => setFilters({ ...filters, [key]: value === '' ? undefined : value })

  const exportCsv = async () => {
    setExporting(true)
    try {
      const allRuns = await fetchAllRuns(
        dateFrom === undefined ? {} : { dateFrom }
      )
      const exported = filterActivity(
        activityRows(allRuns, delegation.history, agentName),
        filters,
        Date.now()
      )
      downloadText(
        exportFilename('nandi-activity', 'csv'),
        activityToCsv(exported),
        'text/csv'
      )
    } catch (e) {
      toastError(e)
    } finally {
      setExporting(false)
    }
  }

  const select = (
    label: string,
    value: string | undefined,
    options: { value: string; label: string }[],
    onChange: (value: string) => void
  ) => (
    <NativeSelect
      aria-label={label}
      value={value ?? ''}
      onChange={event => onChange(event.target.value)}
      className="h-8"
    >
      <NativeSelectOption value="">{label}</NativeSelectOption>
      {options.map(option => (
        <NativeSelectOption key={option.value} value={option.value}>
          {option.label}
        </NativeSelectOption>
      ))}
    </NativeSelect>
  )

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        leading={
          <Button
            size="sm"
            variant="ghost"
            onClick={() =>
              useNavigationStore.getState().navigate({ kind: 'home' })
            }
          >
            <ArrowLeft className="rtl:rotate-180" />
            {t('nav.home')}
          </Button>
        }
        title={t('home.allActivity')}
        actions={
          <Button
            size="sm"
            variant="outline"
            disabled={rows.length === 0 || exporting}
            onClick={() => void exportCsv()}
          >
            <Download />
            {t('home.activity.export')}
          </Button>
        }
      />
      <div className="flex shrink-0 flex-wrap items-center gap-2 border-b px-4 py-2">
        <Input
          value={filters.search ?? ''}
          onChange={event => set('search', event.target.value)}
          placeholder={t('home.activity.search')}
          aria-label={t('home.activity.search')}
          className="h-8 w-52"
        />
        <NativeSelect
          aria-label={t('home.activity.range')}
          value={filters.range ?? '7d'}
          onChange={event => set('range', event.target.value as ActivityRange)}
          className="h-8"
        >
          {RANGES.map(range => (
            <NativeSelectOption key={range} value={range}>
              {t(`home.activity.rangeOption.${range}`)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        {select(
          t('home.activity.status'),
          filters.status,
          STATUSES.map(s => ({ value: s, label: t(`home.status.${s}`) })),
          value => set('status', value as ActivityStatus)
        )}
        {select(
          t('home.activity.source'),
          filters.source,
          SOURCES.map(s => ({
            value: s,
            label: t(`home.activity.sourceOption.${s}`),
          })),
          value => set('source', value as ActivitySource)
        )}
        {select(
          t('home.activity.direction'),
          filters.direction,
          DIRECTIONS.map(d => ({
            value: d,
            label: t(`home.activity.directionOption.${d}`),
          })),
          value => set('direction', value as ActivityDirection)
        )}
        {select(
          t('home.activity.agent'),
          filters.agent,
          facets.agents.map(a => ({ value: a, label: a })),
          value => set('agent', value)
        )}
        {select(
          t('home.activity.member'),
          filters.member,
          facets.members.map(m => ({ value: m, label: m })),
          value => set('member', value)
        )}
        {filtered && (
          <Button size="sm" variant="ghost" onClick={() => setFilters({})}>
            {t('home.activity.clear')}
          </Button>
        )}
      </div>
      <ScrollArea className="min-h-0 flex-1">
        <div className="flex flex-col gap-3 p-4">
          {runs.isPending ? (
            <div className="flex flex-col gap-2" data-testid="activity-loading">
              <Skeleton className="h-8 w-full" />
              <Skeleton className="h-8 w-full" />
            </div>
          ) : rows.length === 0 ? (
            <div className="flex flex-wrap items-center gap-3 py-6 text-sm text-muted-foreground">
              {filtered ? (
                <>
                  {t('home.activity.emptyFiltered')}
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => setFilters({})}
                  >
                    {t('home.activity.clear')}
                  </Button>
                </>
              ) : (
                <>
                  {t('home.activity.empty')}
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() =>
                      useNavigationStore
                        .getState()
                        .navigate({ kind: 'studio', conversationId: null })
                    }
                  >
                    {t('home.askLabel')}
                  </Button>
                </>
              )}
            </div>
          ) : (
            <>
              <p
                className="text-sm text-muted-foreground"
                data-testid="activity-summary"
              >
                {t('home.activity.summary', {
                  count: summary.total,
                  done: summary.done,
                  failed: summary.failed,
                  tokens: formatTokenCount(summary.tokens, i18n.language),
                  cost: formatCostUsd(summary.costUsd, i18n.language),
                })}
              </p>
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b text-xs text-muted-foreground">
                    <th className="py-2 pe-3 text-start font-medium">
                      {t('home.activity.col.when')}
                    </th>
                    <th className="py-2 pe-3 text-start font-medium">
                      {t('home.activity.col.request')}
                    </th>
                    <th className="py-2 pe-3 text-start font-medium">
                      {t('home.activity.col.source')}
                    </th>
                    <th className="py-2 pe-3 text-start font-medium">
                      {t('home.activity.col.who')}
                    </th>
                    <th className="py-2 pe-3 text-end font-medium">
                      {t('home.activity.col.took')}
                    </th>
                    <th className="py-2 text-start font-medium">
                      {t('home.activity.col.state')}
                    </th>
                  </tr>
                </thead>
                <tbody className="divide-y">
                  {rows.slice(0, shown).map(row => (
                    <tr
                      key={row.id}
                      className="cursor-pointer hover:bg-accent/40"
                      onClick={() =>
                        useNavigationStore.getState().navigate(row.view)
                      }
                    >
                      <td className="py-2 pe-3 whitespace-nowrap text-muted-foreground">
                        {formatRelativeTime(row.startedAt, i18n.language)}
                      </td>
                      <td className="max-w-80 truncate py-2 pe-3">
                        <button
                          type="button"
                          className="truncate text-start hover:underline"
                          onClick={event => {
                            event.stopPropagation()
                            useNavigationStore.getState().navigate(row.view)
                          }}
                        >
                          {row.title}
                        </button>
                      </td>
                      <td className="py-2 pe-3">
                        <SourceChip source={activityHomeRow(row).source} />
                      </td>
                      <td className="max-w-40 truncate py-2 pe-3 text-muted-foreground">
                        {row.agent ?? row.member ?? '—'}
                      </td>
                      <td className="py-2 pe-3 text-end tabular-nums text-muted-foreground">
                        {row.endedAt === null
                          ? '—'
                          : formatDuration(row.endedAt - row.startedAt)}
                      </td>
                      <td className="py-2">{t(`home.status.${row.status}`)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
              {rows.length > shown && (
                <Button
                  variant="ghost"
                  className="self-center"
                  onClick={() => setShown(count => count + PAGE)}
                >
                  {t('home.activity.loadMore')}
                </Button>
              )}
            </>
          )}
        </div>
      </ScrollArea>
    </div>
  )
}
