import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQueryClient } from '@tanstack/react-query'
import {
  ChevronLeft,
  ChevronRight,
  Download,
  RefreshCw,
  ScrollText,
  Search,
} from 'lucide-react'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
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
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import {
  AUDIT_KINDS,
  AUDIT_PAGE_SIZE,
  auditQueryKeys,
  entriesToJsonl,
  fetchAuditLog,
  matchesAuditSearch,
  useAuditLog,
  type AuditColumn,
  type AuditKind,
} from '@/hooks/useAuditLog'
import { downloadText, exportFilename } from '@/hooks/useRuns'
import { formatDuration, formatRelativeTime } from '@/lib/format-time'
import type { AuditEntry } from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'
import { useNavigationStore, type AuditLogTab } from '@/store/navigation-store'
import { ViewHeader } from './ViewHeader'

export interface AuditLogViewProps {
  tab?: AuditLogTab
}

const COLUMNS: AuditColumn[] = ['all', 'agent', 'tool', 'outcome', 'details']

const SKELETON_ROWS = 6

/** Calls are often sub-second: "850 ms", then "2m 10s" style */
function formatAuditDuration(ms: number): string {
  return ms < 1000 ? `${Math.round(ms)} ms` : formatDuration(ms)
}

/** Outcomes that mean something went wrong or was refused */
const BAD_OUTCOMES = new Set([
  'error',
  'denied',
  'failed',
  'rate_limited',
  'skipped',
])

/**
 * Settings › Logs: model calls, internet, trigger, pilot and connector
 * logs. Entries are
 * redacted by the backend and again here.
 */
export function AuditLogView({ tab = 'llm' }: AuditLogViewProps) {
  const { t } = useTranslation()

  const handleTab = (value: string) =>
    useNavigationStore.getState().replace({
      kind: 'settings',
      section: 'logs',
      logTab: value as AuditLogTab,
    })

  return (
    <Tabs
      value={tab}
      onValueChange={handleTab}
      className="flex h-full min-h-0 flex-col gap-0"
    >
      <ViewHeader
        title={t('audit.title')}
        subtitle={t('audit.subtitle')}
        leading={<ScrollText className="size-4 text-muted-foreground" />}
        actions={
          <TabsList>
            {AUDIT_KINDS.map(kind => (
              <TabsTrigger key={kind} value={kind}>
                {t(`audit.tab.${kind}`)}
              </TabsTrigger>
            ))}
          </TabsList>
        }
      />
      {AUDIT_KINDS.map(kind => (
        <TabsContent key={kind} value={kind} className="min-h-0 flex-1">
          <ScrollArea className="h-full">
            <AuditLogPanel kind={kind} />
          </ScrollArea>
        </TabsContent>
      ))}
    </Tabs>
  )
}

function AuditLogPanel({ kind }: { kind: AuditKind }) {
  const { t } = useTranslation()
  const queryClient = useQueryClient()
  const [page, setPage] = useState(0)
  const [search, setSearch] = useState('')
  const [column, setColumn] = useState<AuditColumn>('all')
  const [downloading, setDownloading] = useState(false)
  const { data, isLoading, isFetching, error } = useAuditLog(kind, page)

  const entries = data ?? []
  const visible = entries.filter(entry =>
    matchesAuditSearch(entry, search, column)
  )
  const hasNext = entries.length === AUDIT_PAGE_SIZE

  const handleDownload = async () => {
    setDownloading(true)
    try {
      const all = await fetchAuditLog(kind)
      downloadText(
        exportFilename(`nandi-audit-${kind}`, 'jsonl'),
        entriesToJsonl(all),
        'application/x-ndjson'
      )
    } catch (e) {
      toast.error(t('audit.downloadFailed', { error: String(e) }))
    } finally {
      setDownloading(false)
    }
  }

  return (
    <div className="mx-auto flex w-full max-w-6xl flex-col gap-4 p-6">
      <div className="flex flex-wrap items-center gap-2">
        <div className="relative min-w-48 flex-1">
          <Search
            aria-hidden
            className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
          />
          <Input
            type="search"
            value={search}
            onChange={event => setSearch(event.target.value)}
            placeholder={t('audit.search')}
            aria-label={t('audit.search')}
            className="ps-8"
          />
        </div>
        <NativeSelect
          aria-label={t('audit.column.label')}
          value={column}
          onChange={event => setColumn(event.target.value as AuditColumn)}
        >
          {COLUMNS.map(option => (
            <NativeSelectOption key={option} value={option}>
              {t(`audit.column.${option}`)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
        <Button
          size="sm"
          variant="outline"
          disabled={isFetching}
          onClick={() =>
            void queryClient.invalidateQueries({
              queryKey: auditQueryKeys.kind(kind),
            })
          }
        >
          <RefreshCw className={cn(isFetching && 'animate-spin')} />
          {t('audit.refresh')}
        </Button>
        <Button
          size="sm"
          variant="outline"
          disabled={downloading || entries.length === 0}
          onClick={() => void handleDownload()}
        >
          <Download />
          {t('audit.download')}
        </Button>
      </div>

      {error && (
        <p role="alert" className="text-sm text-destructive">
          {String(error)}
        </p>
      )}

      {isLoading ? (
        <div
          className="flex flex-col gap-2"
          aria-busy="true"
          aria-label={t('audit.loading')}
        >
          {Array.from({ length: SKELETON_ROWS }, (_, i) => (
            <Skeleton key={i} className="h-10 w-full" />
          ))}
        </div>
      ) : visible.length === 0 ? (
        <Empty className="border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <ScrollText />
            </EmptyMedia>
            <EmptyTitle>
              {t(
                entries.length === 0 ? 'audit.empty.title' : 'audit.noMatches'
              )}
            </EmptyTitle>
            {entries.length === 0 && (
              <EmptyDescription>{t(`audit.empty.${kind}`)}</EmptyDescription>
            )}
          </EmptyHeader>
        </Empty>
      ) : (
        <AuditTable kind={kind} entries={visible} />
      )}

      {(page > 0 || hasNext) && (
        <nav
          aria-label={t('audit.pagination')}
          className="flex items-center justify-end gap-1 text-xs text-muted-foreground"
        >
          <Button
            size="icon-sm"
            variant="ghost"
            disabled={page === 0}
            onClick={() => setPage(page - 1)}
            aria-label={t('audit.previous')}
            title={t('audit.previous')}
          >
            <ChevronLeft className="rtl:rotate-180" />
          </Button>
          <span className="tabular-nums">
            {t('audit.page', { page: page + 1 })}
          </span>
          <Button
            size="icon-sm"
            variant="ghost"
            disabled={!hasNext}
            onClick={() => setPage(page + 1)}
            aria-label={t('audit.next')}
            title={t('audit.next')}
          >
            <ChevronRight className="rtl:rotate-180" />
          </Button>
        </nav>
      )}
    </div>
  )
}

function AuditTable({
  kind,
  entries,
}: {
  kind: AuditKind
  entries: AuditEntry[]
}) {
  const { t, i18n } = useTranslation()
  return (
    <div className="overflow-x-auto rounded-lg border">
      <table className="w-full text-sm">
        <thead className="bg-muted/40 text-xs text-muted-foreground">
          <tr className="[&_th]:px-3 [&_th]:py-2 [&_th]:text-start [&_th]:font-medium [&_th]:whitespace-nowrap">
            <th scope="col">{t('audit.columns.time')}</th>
            <th scope="col">{t('audit.columns.agent')}</th>
            <th scope="col">{t(`audit.columns.tool.${kind}`)}</th>
            <th scope="col">{t('audit.columns.outcome')}</th>
            <th scope="col">{t('audit.columns.duration')}</th>
            <th scope="col">{t('audit.columns.details')}</th>
          </tr>
        </thead>
        <tbody>
          {entries.map((entry, index) => {
            const details =
              entry.argsRedacted === null
                ? ''
                : JSON.stringify(entry.argsRedacted)
            return (
              <tr
                // Lines have no ID; time + position is stable within a page
                key={`${entry.timestamp}-${index}`}
                className="border-t [&_td]:px-3 [&_td]:py-2"
              >
                <td
                  className="text-xs whitespace-nowrap text-muted-foreground"
                  title={new Date(entry.timestamp).toLocaleString(
                    i18n.language
                  )}
                >
                  <time dateTime={new Date(entry.timestamp).toISOString()}>
                    {formatRelativeTime(entry.timestamp, i18n.language)}
                  </time>
                </td>
                <td className="text-xs whitespace-nowrap">
                  {entry.agentId ?? '—'}
                </td>
                <td className="max-w-56 truncate font-mono text-xs" dir="ltr">
                  {entry.tool ?? '—'}
                </td>
                <td>
                  <Badge
                    variant="outline"
                    className={cn(
                      BAD_OUTCOMES.has(entry.outcome)
                        ? 'border-destructive/30 bg-destructive/10 text-destructive'
                        : 'text-muted-foreground'
                    )}
                  >
                    {entry.outcome}
                  </Badge>
                </td>
                <td className="text-xs whitespace-nowrap tabular-nums">
                  {entry.durationMs === null
                    ? '—'
                    : formatAuditDuration(entry.durationMs)}
                </td>
                <td
                  className="max-w-96 truncate font-mono text-xs text-muted-foreground"
                  dir="ltr"
                  title={details}
                >
                  {details}
                </td>
              </tr>
            )
          })}
        </tbody>
      </table>
    </div>
  )
}
