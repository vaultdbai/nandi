import { useTranslation } from 'react-i18next'
import { Skeleton } from '@/components/ui/skeleton'
import { RunChart } from '@/components/agent-console/RunChart'
import { useCapabilityEvidence } from '@/hooks/useCapabilityEvidence'
import { errorMessage } from '@/hooks/useDelegation'
import { AmbiguityList } from './AmbiguityList'
import {
  formatDuration,
  formatRate,
  formatUsd,
  totalEvidence,
} from './capability-model'

/**
 * The trend windows. The core aggregates evidence per window, not per
 * day, so the "30 days" view compares the last day, week, and month.
 */
const TREND_WINDOWS = [1, 7, 30] as const

/**
 * Aggregate cards (invocations, success rate, p50, cost), invocations and
 * success rate over the last day, week, and 30 days, and the clarification
 * questions that contribute most.
 */
export function EvidencePanel({
  capabilityId,
  canPromote = true,
}: {
  capabilityId: string
  canPromote?: boolean
}) {
  const { t } = useTranslation()
  const day = useCapabilityEvidence(capabilityId, 1)
  const week = useCapabilityEvidence(capabilityId, 7)
  const month = useCapabilityEvidence(capabilityId, 30)

  if (month.isPending) {
    return (
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        {[0, 1, 2, 3].map(i => (
          <Skeleton key={i} className="h-20" />
        ))}
      </div>
    )
  }
  if (month.error) {
    return (
      <p className="text-sm text-destructive">
        {t('capabilities.loadError', { error: errorMessage(month.error) })}
      </p>
    )
  }

  const totals = totalEvidence(month.data)
  const trend = [day, week, month].map((query, index) => ({
    days: TREND_WINDOWS[index] ?? 30,
    totals: query.data ? totalEvidence(query.data) : null,
  }))
  const windowLabel = (days: number) => t(`capabilities.evidence.window${days}`)

  return (
    <div className="flex flex-col gap-6" data-testid="evidence-panel">
      <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
        <AggregateCard
          label={t('capabilities.evidence.invocations')}
          value={String(totals.count)}
        />
        <AggregateCard
          label={t('capabilities.evidence.successRate')}
          value={formatRate(totals.successRate)}
        />
        <AggregateCard
          label={t('capabilities.evidence.p50')}
          value={formatDuration(totals.p50Ms)}
        />
        <AggregateCard
          label={t('capabilities.evidence.cost')}
          value={formatUsd(totals.costUsd)}
        />
      </div>
      {totals.count === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t('capabilities.evidence.none')}
        </p>
      ) : (
        <div className="grid gap-6 md:grid-cols-2">
          <RunChart
            title={t('capabilities.evidence.invocationsTrend')}
            points={trend.map(({ days, totals }) => ({
              label: windowLabel(days),
              value: totals?.count ?? 0,
              display: String(totals?.count ?? 0),
            }))}
          />
          <RunChart
            title={t('capabilities.evidence.successTrend')}
            points={trend.map(({ days, totals }) => ({
              label: windowLabel(days),
              value: Math.round((totals?.successRate ?? 0) * 100),
              display: formatRate(totals?.successRate ?? null),
            }))}
          />
        </div>
      )}
      <section className="flex flex-col gap-2">
        <h3 className="text-sm font-medium">
          {t('capabilities.evidence.ambiguities')}
        </h3>
        <AmbiguityList
          capabilityId={capabilityId}
          limit={5}
          canPromote={canPromote}
        />
      </section>
    </div>
  )
}

function AggregateCard({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex flex-col gap-1 rounded-lg border p-3">
      <span className="text-xs text-muted-foreground">{label}</span>
      <span className="text-xl font-semibold tabular-nums">{value}</span>
    </div>
  )
}
