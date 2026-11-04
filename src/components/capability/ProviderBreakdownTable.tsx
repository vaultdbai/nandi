import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ArrowDown, ArrowUp, Search } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Skeleton } from '@/components/ui/skeleton'
import { useCapabilityProviderBreakdown } from '@/hooks/useCapabilityEvidence'
import { errorMessage } from '@/hooks/useDelegation'
import type { CapabilityProviderStats, ProviderKind } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { useNavigationStore, type View } from '@/store/navigation-store'
import {
  filterBreakdown,
  formatDuration,
  formatRate,
  formatUsd,
  providerLabel,
  sortBreakdown,
  successRateOf,
  type BreakdownSort,
  type BreakdownSortKey,
} from './capability-model'

const COLUMNS: BreakdownSortKey[] = [
  'provider',
  'kind',
  'status',
  'load',
  'successRate',
  'p50',
  'p95',
  'calls',
  'cost',
]

/** Where a row leads: the local agent or workflow, else the member */
function providerView(row: CapabilityProviderStats): View {
  const { provider } = row
  if (!provider.local) {
    return { kind: 'member-profile', instanceId: provider.instanceId }
  }
  return provider.providerKind === 'workflow'
    ? { kind: 'workflow', workflowId: provider.providerId }
    : { kind: 'agent', agentId: provider.providerId }
}

/**
 * One row per provider (instance, kind, status, load, success rate,
 * p50/p95, calls, cost); sortable by any column, filterable by name and
 * kind; a row opens the agent, workflow, or member.
 */
export function ProviderBreakdownTable({
  capabilityId,
}: {
  capabilityId: string
}) {
  const { t } = useTranslation()
  const breakdown = useCapabilityProviderBreakdown(capabilityId)
  const [query, setQuery] = useState('')
  const [kind, setKind] = useState<ProviderKind | 'all'>('all')
  const [sort, setSort] = useState<BreakdownSort>({
    key: 'calls',
    direction: 'desc',
  })

  if (breakdown.isPending) return <Skeleton className="h-32" />
  if (breakdown.error) {
    return (
      <p className="text-sm text-destructive">
        {t('capabilities.loadError', { error: errorMessage(breakdown.error) })}
      </p>
    )
  }

  const rows = sortBreakdown(
    filterBreakdown(breakdown.data, query).filter(
      row => kind === 'all' || row.provider.providerKind === kind
    ),
    sort
  )

  const toggleSort = (key: BreakdownSortKey) =>
    setSort(current =>
      current.key === key
        ? { key, direction: current.direction === 'asc' ? 'desc' : 'asc' }
        : {
            key,
            direction: key === 'provider' || key === 'kind' ? 'asc' : 'desc',
          }
    )

  const open = (row: CapabilityProviderStats) =>
    useNavigationStore.getState().navigate(providerView(row))

  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <div className="relative min-w-48 flex-1">
          <Search
            className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
            aria-hidden
          />
          <Input
            value={query}
            onChange={event => setQuery(event.target.value)}
            placeholder={t('capabilities.providers.filter')}
            aria-label={t('capabilities.providers.filter')}
            className="ps-8"
          />
        </div>
        <NativeSelect
          value={kind}
          aria-label={t('capabilities.providers.kindFilter')}
          onChange={event =>
            setKind(event.target.value as ProviderKind | 'all')
          }
        >
          <NativeSelectOption value="all">
            {t('capabilities.providers.allKinds')}
          </NativeSelectOption>
          <NativeSelectOption value="agent">
            {t('capabilities.providerKind.agent')}
          </NativeSelectOption>
          <NativeSelectOption value="workflow">
            {t('capabilities.providerKind.workflow')}
          </NativeSelectOption>
        </NativeSelect>
      </div>
      {breakdown.data.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t('capabilities.providers.empty')}
        </p>
      ) : (
        <div className="overflow-x-auto rounded-md border">
          <table className="w-full text-sm">
            <thead className="bg-muted/50 text-xs text-muted-foreground">
              <tr>
                {COLUMNS.map(column => (
                  <th
                    key={column}
                    scope="col"
                    className="px-3 py-2 text-start font-medium"
                    aria-sort={
                      sort.key === column
                        ? sort.direction === 'asc'
                          ? 'ascending'
                          : 'descending'
                        : 'none'
                    }
                  >
                    <button
                      type="button"
                      className="inline-flex items-center gap-1 hover:text-foreground"
                      onClick={() => toggleSort(column)}
                    >
                      {t(`capabilities.providers.column.${column}`)}
                      {sort.key === column &&
                        (sort.direction === 'asc' ? (
                          <ArrowUp className="size-3" aria-hidden />
                        ) : (
                          <ArrowDown className="size-3" aria-hidden />
                        ))}
                    </button>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.map(row => (
                <tr
                  key={`${row.provider.instanceId}/${row.provider.providerId}`}
                  data-testid={`provider-row-${row.provider.providerId}`}
                  tabIndex={0}
                  className="cursor-pointer border-t hover:bg-accent/40 focus-visible:bg-accent/40 focus-visible:outline-none"
                  onClick={() => open(row)}
                  onKeyDown={event => {
                    if (event.key === 'Enter') open(row)
                  }}
                >
                  <td className="px-3 py-2">
                    <div className="font-medium">{providerLabel(row)}</div>
                    <div className="text-xs text-muted-foreground">
                      {row.provider.local
                        ? t('capabilities.providers.thisNandi')
                        : row.provider.instanceId}
                    </div>
                  </td>
                  <td className="px-3 py-2">
                    <Badge variant="outline">
                      {t(
                        `capabilities.providerKind.${row.provider.providerKind}`
                      )}
                    </Badge>
                  </td>
                  <td className="px-3 py-2">
                    <span className="inline-flex items-center gap-1.5">
                      <span
                        aria-hidden
                        className={cn(
                          'size-2 rounded-full',
                          row.online ? 'bg-success' : 'bg-muted-foreground/40'
                        )}
                      />
                      {row.online
                        ? t('capabilities.providers.online')
                        : t('capabilities.providers.offline')}
                    </span>
                  </td>
                  <td className="px-3 py-2 tabular-nums">
                    {row.provider.maxConcurrent
                      ? `${row.load} / ${row.provider.maxConcurrent}`
                      : row.load}
                  </td>
                  <td className="px-3 py-2 tabular-nums">
                    {formatRate(successRateOf(row.evidence))}
                  </td>
                  <td className="px-3 py-2 tabular-nums">
                    {formatDuration(row.evidence.p50Ms)}
                  </td>
                  <td className="px-3 py-2 tabular-nums">
                    {formatDuration(row.evidence.p95Ms)}
                  </td>
                  <td className="px-3 py-2 tabular-nums">
                    {row.evidence.count}
                  </td>
                  <td className="px-3 py-2 tabular-nums">
                    {formatUsd(row.evidence.costUsdTotal)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {rows.length === 0 && (
            <p className="border-t px-3 py-4 text-sm text-muted-foreground">
              {t('capabilities.providers.noMatch')}
            </p>
          )}
        </div>
      )}
    </div>
  )
}
