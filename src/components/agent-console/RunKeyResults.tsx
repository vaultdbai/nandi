/**
 * Key result cards: the run's real stats, plus numbers its output stated
 * (cards and a chart) when they can be read reliably; otherwise a text
 * summary. Then the files the run generated.
 */
import { useTranslation } from 'react-i18next'
import { ChevronRight, FileText } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { useRunDetail } from '@/hooks/useSnapshots'
import { formatDuration } from '@/lib/format-time'
import { formatCostUsd, formatTokenCount } from '@/lib/format-usage'
import { MarkdownContent } from './MarkdownContent'
import { RunChart } from './RunChart'
import {
  detectNumericData,
  formatBytes,
  finalOutput,
  generatedFiles,
  runStats,
  type FileEntry,
} from './run-detail-model'

const MAX_VALUE_CARDS = 8

function StatCard({
  label,
  value,
  hint,
}: {
  label: string
  value: React.ReactNode
  hint?: React.ReactNode
}) {
  return (
    <div className="flex min-w-0 flex-col gap-1 rounded-lg border bg-card p-3">
      <dt className="truncate text-xs text-muted-foreground" title={label}>
        {label}
      </dt>
      <dd className="truncate text-xl font-semibold tabular-nums">{value}</dd>
      {hint && (
        <dd className="truncate text-xs text-muted-foreground">{hint}</dd>
      )}
    </div>
  )
}

function GeneratedFiles({
  files,
  onViewFiles,
}: {
  files: FileEntry[]
  onViewFiles?: () => void
}) {
  const { t, i18n } = useTranslation()
  return (
    <Collapsible className="rounded-lg border">
      <div className="flex items-center justify-between gap-2 px-3 py-2">
        <CollapsibleTrigger className="group flex items-center gap-2 text-sm font-medium">
          <ChevronRight className="size-4 transition-transform group-data-[state=open]:rotate-90 rtl:rotate-180 rtl:group-data-[state=open]:rotate-90" />
          {t('runDetail.overview.generatedFiles', { count: files.length })}
        </CollapsibleTrigger>
        {onViewFiles && (
          <Button size="sm" variant="ghost" onClick={onViewFiles}>
            {t('runDetail.overview.viewAllFiles')}
          </Button>
        )}
      </div>
      <CollapsibleContent>
        {files.length === 0 ? (
          <p className="px-3 pb-3 text-xs text-muted-foreground">
            {t('runDetail.overview.noGeneratedFiles')}
          </p>
        ) : (
          <ul className="flex flex-col gap-1 px-3 pb-3">
            {files.map(file => (
              <li key={file.key} className="flex items-center gap-2 text-sm">
                <FileText className="size-4 shrink-0 text-muted-foreground" />
                <code dir="ltr" className="truncate font-mono text-xs">
                  {file.path}
                </code>
                {file.sizeBytes !== null && (
                  <span className="ms-auto shrink-0 text-xs text-muted-foreground tabular-nums">
                    {formatBytes(file.sizeBytes, i18n.language)}
                  </span>
                )}
              </li>
            ))}
          </ul>
        )}
      </CollapsibleContent>
    </Collapsible>
  )
}

export function RunKeyResults({
  runId,
  onViewFiles,
}: {
  runId: string
  /** "View all files" (switches to the Files tab) */
  onViewFiles?: () => void
}) {
  const { t, i18n } = useTranslation()
  const { data: detail } = useRunDetail(runId)
  if (!detail) return null

  const stats = runStats(detail)
  const output = finalOutput(detail)
  const numbers = detectNumericData(output)
  const files = generatedFiles(detail)
  const lng = i18n.language

  return (
    <div className="flex flex-col gap-4">
      <section className="flex flex-col gap-2">
        <h3 className="text-sm font-semibold">
          {t('runDetail.overview.keyResults')}
        </h3>
        <dl className="grid grid-cols-2 gap-2 sm:grid-cols-3 xl:grid-cols-5">
          <StatCard
            label={t('runDetail.stats.duration')}
            value={
              stats.durationMs === null ? '—' : formatDuration(stats.durationMs)
            }
          />
          <StatCard
            label={t('runDetail.stats.tokens')}
            value={formatTokenCount(
              stats.inputTokens + stats.outputTokens,
              lng
            )}
            hint={t('runs.usage.tokens', {
              input: formatTokenCount(stats.inputTokens, lng),
              output: formatTokenCount(stats.outputTokens, lng),
            })}
          />
          <StatCard
            label={t('runDetail.stats.cost')}
            value={
              stats.costUsd === null ? '—' : formatCostUsd(stats.costUsd, lng)
            }
          />
          <StatCard
            label={t('runDetail.stats.toolCalls')}
            value={stats.toolCalls}
            hint={
              stats.failedToolCalls > 0
                ? t('runDetail.stats.failedToolCalls', {
                    count: stats.failedToolCalls,
                  })
                : undefined
            }
          />
          <StatCard
            label={t('runDetail.stats.filesWritten')}
            value={stats.filesWritten}
          />
        </dl>
      </section>

      {numbers ? (
        <section
          aria-label={t('runDetail.overview.fromOutput')}
          className="flex flex-col gap-3 rounded-lg border p-3"
        >
          <h3 className="text-sm font-semibold">
            {t('runDetail.overview.fromOutput')}
          </h3>
          {numbers.points.length <= MAX_VALUE_CARDS && (
            <dl className="grid grid-cols-2 gap-2 sm:grid-cols-4">
              {numbers.points.map((point, i) => (
                <StatCard
                  key={`${point.label}-${i}`}
                  label={point.label}
                  value={point.display}
                />
              ))}
            </dl>
          )}
          {numbers.points.length >= 2 && (
            <RunChart
              title={numbers.title ?? t('runDetail.overview.chartTitle')}
              points={numbers.points}
            />
          )}
        </section>
      ) : (
        <section className="flex flex-col gap-1 rounded-lg border p-3">
          <h3 className="text-sm font-semibold">
            {t('runDetail.overview.summary')}
          </h3>
          {output ? (
            <div className="line-clamp-[12] text-sm">
              <MarkdownContent content={output} />
            </div>
          ) : (
            <p className="text-sm text-muted-foreground">
              {t('runDetail.overview.noOutput')}
            </p>
          )}
        </section>
      )}

      <GeneratedFiles files={files} onViewFiles={onViewFiles} />
    </div>
  )
}
