/**
 * Home's work as one kind of row: agent runs and delegations (both ways)
 * become activity rows with a plain source, direction and state, so Home's
 * Running / Recent and All activity filter and export one list. Pure.
 * Phase 1 of the Request model (work-model §4): rows link to Request
 * detail where one exists (delegations), else to Run detail.
 */
import type {
  RunStatus,
  RunSummary,
  SourceType,
  TaskResult,
} from '@/lib/bindings'
import { STUDIO_AGENT_ID } from '@/lib/agent-identity'
import type { View } from '@/store/navigation-store'

export type ActivityStatus =
  'running' | 'waiting' | 'done' | 'failed' | 'cancelled'
export type ActivitySource =
  'you' | 'trigger' | 'schedule' | 'workflow' | 'peer'
export type ActivityDirection = 'local' | 'incoming' | 'outgoing'
export type ActivityRange = 'today' | '7d' | '30d' | 'all'

export interface ActivityRow {
  id: string
  /** First line of what was asked (never an ID) */
  title: string
  status: ActivityStatus
  source: ActivitySource
  direction: ActivityDirection
  /** The agent that did the work, by name */
  agent: string | null
  /** Its ID (old deep links filter by ID) */
  agentId: string | null
  /** The other person, for delegations */
  member: string | null
  startedAt: number
  endedAt: number | null
  /** One line about the outcome (a run's summary, a task's error) */
  detail: string | null
  costUsd: number | null
  tokens: number
  view: View
}

export interface ActivityFilters {
  search?: string
  range?: ActivityRange
  status?: ActivityStatus
  source?: ActivitySource
  direction?: ActivityDirection
  agent?: string
  member?: string
}

const DAY_MS = 24 * 60 * 60 * 1000

const RUN_STATUS: Record<RunStatus, ActivityStatus> = {
  queued: 'running',
  running: 'running',
  awaitingApproval: 'waiting',
  completed: 'done',
  compacted: 'done',
  failed: 'failed',
  cancelled: 'cancelled',
}

const RUN_SOURCE: Record<SourceType, ActivitySource> = {
  user: 'you',
  workflow: 'workflow',
  agent: 'peer',
  scheduled: 'schedule',
  cron: 'schedule',
  calendar: 'schedule',
  file: 'trigger',
  webhook: 'trigger',
  device: 'trigger',
  event: 'trigger',
  email: 'trigger',
}

const TASK_STATUS: Record<TaskResult['status'], ActivityStatus> = {
  running: 'running',
  completed: 'done',
  failed: 'failed',
  timedOut: 'failed',
  cancelled: 'cancelled',
}

/** The first non-empty line, trimmed and kept short */
export function firstLine(text: string, max = 120): string {
  const line =
    text
      .split(/\r?\n/)
      .map(l => l.trim())
      .find(Boolean) ?? ''
  return line.length > max ? `${line.slice(0, max - 1)}…` : line
}

/** `owner/agent` → `agent` (card IDs) */
function agentPart(cardId: string): string {
  const slash = cardId.indexOf('/')
  return slash === -1 ? cardId : cardId.slice(slash + 1)
}

function inputText(input: TaskResult['input']): string {
  if (typeof input === 'string') return input
  if (input && typeof input === 'object' && !Array.isArray(input)) {
    for (const key of ['text', 'request', 'prompt', 'input']) {
      const value = (input as Record<string, unknown>)[key]
      if (typeof value === 'string') return value
    }
  }
  return ''
}

/** Capability IDs in words: `summarize-paper` → `Summarize paper` */
export function capabilityLabel(capability: string): string {
  const words = capability.replace(/[._-]+/g, ' ').trim()
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : capability
}

/**
 * A run as a row. Studio's own runs and runs a delegation started
 * (source `agent`, shown as that delegation's row) are left out.
 */
export function runRow(
  run: RunSummary,
  agentName: (id: string) => string
): ActivityRow | null {
  if (run.agentId === STUDIO_AGENT_ID || run.sourceType === 'agent') return null
  return {
    id: `run:${run.id}`,
    title: firstLine(run.input) || agentName(run.agentId),
    status: RUN_STATUS[run.status],
    source: RUN_SOURCE[run.sourceType],
    direction: 'local',
    agent: agentName(run.agentId),
    agentId: run.agentId,
    member: null,
    startedAt: run.startedAt,
    endedAt: run.endedAt,
    detail: run.summary ? firstLine(run.summary) : null,
    costUsd: run.costUsd,
    tokens: run.inputTokens + run.outputTokens,
    view: { kind: 'run-detail', runId: run.id },
  }
}

/** A delegation (either way) as a row; it opens its Request */
export function delegationRow(row: TaskResult): ActivityRow {
  const outgoing = row.direction === 'outbound'
  const card = outgoing ? row.providerAgent : row.requesterAgent
  const owner = outgoing ? row.providerOwner : row.requesterOwner
  return {
    id: `task:${row.taskId}`,
    title: firstLine(inputText(row.input)) || capabilityLabel(row.capability),
    status: TASK_STATUS[row.status],
    source: outgoing ? 'you' : 'peer',
    direction: outgoing ? 'outgoing' : 'incoming',
    agent: outgoing ? null : agentPart(row.providerAgent),
    agentId: outgoing ? null : agentPart(row.providerAgent),
    member: owner ?? agentPart(card),
    startedAt: row.startedAt,
    endedAt: row.endedAt,
    detail: row.error ? firstLine(row.error) : null,
    costUsd: null,
    tokens: row.tokensIn + row.tokensOut,
    view: { kind: 'request', requestId: row.taskId },
  }
}

/** Runs and delegations in one list, newest first */
export function activityRows(
  runs: RunSummary[],
  delegations: TaskResult[],
  agentName: (id: string) => string
): ActivityRow[] {
  const rows = [
    ...runs.flatMap(run => runRow(run, agentName) ?? []),
    ...delegations.map(delegationRow),
  ]
  const seen = new Set<string>()
  return rows
    .filter(row => !seen.has(row.id) && seen.add(row.id))
    .sort((a, b) => b.startedAt - a.startedAt)
}

/** Local midnight of `now` */
export function startOfDay(now: number): number {
  const day = new Date(now)
  day.setHours(0, 0, 0, 0)
  return day.getTime()
}

/** Where `range` starts (`0`: all time) */
export function rangeStart(range: ActivityRange | undefined, now: number) {
  switch (range ?? '7d') {
    case 'today':
      return startOfDay(now)
    case '7d':
      return now - 7 * DAY_MS
    case '30d':
      return now - 30 * DAY_MS
    case 'all':
      return 0
  }
}

/** Every filter must hold (AND); the date range defaults to 7 days */
export function filterActivity(
  rows: ActivityRow[],
  filters: ActivityFilters,
  now: number
): ActivityRow[] {
  const since = rangeStart(filters.range, now)
  const search = filters.search?.trim().toLowerCase()
  return rows.filter(row => {
    if (row.startedAt < since) return false
    if (filters.status && row.status !== filters.status) return false
    if (filters.source && row.source !== filters.source) return false
    if (filters.direction && row.direction !== filters.direction) return false
    if (
      filters.agent &&
      row.agent !== filters.agent &&
      row.agentId !== filters.agent
    ) {
      return false
    }
    if (filters.member && row.member !== filters.member) return false
    if (search) {
      const haystack = [row.title, row.detail, row.agent, row.member]
        .filter(Boolean)
        .join(' ')
        .toLowerCase()
      if (!haystack.includes(search)) return false
    }
    return true
  })
}

/** Filters other than the default range */
export function hasActivityFilters(filters: ActivityFilters): boolean {
  return Object.entries(filters).some(
    ([key, value]) =>
      value !== undefined &&
      value !== '' &&
      !(key === 'range' && value === '7d')
  )
}

/** Distinct agents and members, sorted, for the filter menus */
export function activityFacets(rows: ActivityRow[]): {
  agents: string[]
  members: string[]
} {
  const sorted = (values: (string | null)[]) =>
    [...new Set(values.filter((v): v is string => !!v))].sort((a, b) =>
      a.localeCompare(b)
    )
  return {
    agents: sorted(rows.map(r => r.agent)),
    members: sorted(rows.map(r => r.member)),
  }
}

export interface ActivitySummary {
  total: number
  done: number
  failed: number
  tokens: number
  costUsd: number
}

export function activitySummary(rows: ActivityRow[]): ActivitySummary {
  return rows.reduce<ActivitySummary>(
    (sum, row) => ({
      total: sum.total + 1,
      done: sum.done + (row.status === 'done' ? 1 : 0),
      failed: sum.failed + (row.status === 'failed' ? 1 : 0),
      tokens: sum.tokens + row.tokens,
      costUsd: sum.costUsd + (row.costUsd ?? 0),
    }),
    { total: 0, done: 0, failed: 0, tokens: 0, costUsd: 0 }
  )
}

const CSV_COLUMNS = [
  'startedAt',
  'endedAt',
  'title',
  'status',
  'source',
  'direction',
  'agent',
  'member',
  'tokens',
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

/** The rows as CSV (header first, ISO 8601 times) */
export function activityToCsv(rows: ActivityRow[]): string {
  const iso = (time: number | null) =>
    time === null ? null : new Date(time).toISOString()
  const lines = rows.map(row =>
    CSV_COLUMNS.map(column => {
      switch (column) {
        case 'startedAt':
          return csvCell(iso(row.startedAt))
        case 'endedAt':
          return csvCell(iso(row.endedAt))
        default:
          return csvCell(row[column])
      }
    }).join(',')
  )
  return [CSV_COLUMNS.join(','), ...lines].join('\r\n') + '\r\n'
}
