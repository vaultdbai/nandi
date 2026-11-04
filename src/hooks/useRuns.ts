import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import {
  unwrapResult,
  type DateRange,
  type RunFilters,
  type RunSummary,
} from '@/lib/tauri-bindings'

/**
 * Paged, filtered runs across every agent, and their stats.
 *
 * No polling: `agent://run`, `agent://turn_done`, `dashboard://activity`,
 * and trigger events invalidate `runsQueryKeys.all` (see `agent-sync`).
 */

/** Rows per page (the server pages; this is one request's worth) */
export const RUNS_PAGE_SIZE = 100

/** The most rows the server returns per request (`MAX_PAGE_SIZE`) */
const MAX_SERVER_PAGE = 200

/** CSV export stops here rather than pulling an unbounded history */
export const MAX_EXPORT_ROWS = 10_000

/** `dateTo` when the range is open-ended (later than any real run) */
export const OPEN_END = Number.MAX_SAFE_INTEGER

export const runsQueryKeys = {
  all: ['globalRuns'] as const,
  page: (filters: RunFilters, limit: number, offset: number) =>
    ['globalRuns', 'page', filters, limit, offset] as const,
  stats: (range: DateRange) => ['globalRuns', 'stats', range] as const,
}

const STRING_FILTERS = ['status', 'sourceType', 'agentId', 'search'] as const

/**
 * Filters with blank values dropped and strings trimmed, so equal filters
 * share one query key and the server never sees `""`.
 */
export function normalizeRunFilters(filters: RunFilters): RunFilters {
  const normalized: RunFilters = {}
  for (const key of STRING_FILTERS) {
    const value = filters[key]?.trim()
    if (value) normalized[key] = value
  }
  for (const key of ['dateFrom', 'dateTo'] as const) {
    const value = filters[key]
    if (typeof value === 'number' && Number.isFinite(value))
      normalized[key] = value
  }
  return normalized
}

/** Whether any filter narrows the result */
export function hasRunFilters(filters: RunFilters): boolean {
  return Object.keys(normalizeRunFilters(filters)).length > 0
}

/** The stats range for `filters`: their dates, open-ended otherwise */
export function statsRange(filters: RunFilters): DateRange {
  return { from: filters.dateFrom ?? 0, to: filters.dateTo ?? OPEN_END }
}

/** One page (0-based) of runs matching `filters`, newest first. */
export function useRuns(
  filters: RunFilters,
  page = 0,
  pageSize = RUNS_PAGE_SIZE
) {
  const normalized = normalizeRunFilters(filters)
  const offset = page * pageSize
  return useQuery({
    queryKey: runsQueryKeys.page(normalized, pageSize, offset),
    queryFn: async () =>
      unwrapResult(await client.listAllRuns(normalized, pageSize, offset)),
    // Keep the current page on screen while the next one loads
    placeholderData: keepPreviousData,
  })
}

/** Every run matching `filters` (up to `max`), page by page. */
export async function fetchAllRuns(
  filters: RunFilters,
  max = MAX_EXPORT_ROWS
): Promise<RunSummary[]> {
  const normalized = normalizeRunFilters(filters)
  const rows: RunSummary[] = []
  while (rows.length < max) {
    const limit = Math.min(MAX_SERVER_PAGE, max - rows.length)
    const page = unwrapResult(
      await client.listAllRuns(normalized, limit, rows.length)
    )
    rows.push(...page.rows)
    if (page.rows.length < limit || rows.length >= page.total) break
  }
  return rows
}

/** `null` until the run ends */
export function runDurationMs(run: RunSummary): number | null {
  return run.endedAt === null ? null : run.endedAt - run.startedAt
}

const CSV_COLUMNS = [
  'id',
  'startedAt',
  'endedAt',
  'durationMs',
  'agentId',
  'agentName',
  'status',
  'sourceType',
  'sourceLabel',
  'input',
  'summary',
  'model',
  'inputTokens',
  'outputTokens',
  'costUsd',
] as const

/** RFC 4180 quoting; formula-leading cells are prefixed so spreadsheets
 * don't evaluate them */
function csvCell(value: string | number | null): string {
  if (value === null) return ''
  let text = String(value)
  if (typeof value === 'string' && /^[=+\-@\t\r]/.test(text)) text = `'${text}`
  return /[",\r\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text
}

/** The runs as CSV (header row first, ISO 8601 times) */
export function runsToCsv(
  runs: RunSummary[],
  agentNames: Record<string, string> = {}
): string {
  const iso = (time: number | null) =>
    time === null ? null : new Date(time).toISOString()
  const lines = runs.map(run => {
    const row: Record<(typeof CSV_COLUMNS)[number], string | number | null> = {
      id: run.id,
      startedAt: iso(run.startedAt),
      endedAt: iso(run.endedAt),
      durationMs: runDurationMs(run),
      agentId: run.agentId,
      agentName: agentNames[run.agentId] ?? null,
      status: run.status,
      sourceType: run.sourceType,
      sourceLabel: run.sourceLabel,
      input: run.input,
      summary: run.summary,
      model: run.model,
      inputTokens: run.inputTokens,
      outputTokens: run.outputTokens,
      costUsd: run.costUsd,
    }
    return CSV_COLUMNS.map(column => csvCell(row[column])).join(',')
  })
  return [CSV_COLUMNS.join(','), ...lines].join('\r\n') + '\r\n'
}

/** Saves `text` through the browser's download (both transports) */
export function downloadText(filename: string, text: string, type: string) {
  const url = URL.createObjectURL(new Blob([text], { type }))
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  link.style.display = 'none'
  document.body.append(link)
  link.click()
  link.remove()
  // Let the download start before the URL goes away
  setTimeout(() => URL.revokeObjectURL(url), 1_000)
}

/** `nandi-runs-2026-10-06.csv` style names */
export function exportFilename(prefix: string, extension: string): string {
  return `${prefix}-${new Date().toISOString().slice(0, 10)}.${extension}`
}
