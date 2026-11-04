/**
 * Pure derivations for the run detail view: timeline steps, files touched,
 * the final output, and numbers found in it. No React, no I/O.
 */
import type { ChatMessage, RunDetail, RunStatus } from '@/lib/tauri-bindings'

// ---------------------------------------------------------------------------
// Timeline

export type StepState = 'done' | 'running' | 'pending' | 'failed'

export interface TimelineStep {
  id: string
  at: number
  state: StepState
  /** i18n key under `runDetail.timeline.step.*` */
  kind:
    | 'started'
    | 'request'
    | 'reply'
    | 'tool'
    | 'approval'
    | 'error'
    | 'finished'
    | 'running'
  /** Tool name, for tool and approval steps */
  tool?: string
  /** One-line description */
  description: string
  /** The run's final status, for the `finished` step */
  status?: RunStatus
  /** Until the next step began (finished steps only) */
  durationMs?: number
}

export function oneLine(text: string, max = 140): string {
  const flat = text.replace(/\s+/g, ' ').trim()
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat
}

function argsLine(args: unknown): string {
  if (args && typeof args === 'object' && !Array.isArray(args)) {
    const record = args as Record<string, unknown>
    const preferred = ['cmd', 'command', 'path', 'file_path', 'query', 'url']
    for (const key of preferred) {
      const value = record[key]
      if (typeof value === 'string') return oneLine(value)
    }
  }
  return oneLine(JSON.stringify(args ?? {}))
}

const FAILED_STATUSES: readonly RunStatus[] = ['failed', 'cancelled']
const ACTIVE_STATUSES: readonly RunStatus[] = [
  'queued',
  'running',
  'awaitingApproval',
]

export function isActive(status: RunStatus): boolean {
  return ACTIVE_STATUSES.includes(status)
}

function messageStep(message: ChatMessage, runActive: boolean): TimelineStep {
  const base = { id: message.id, at: message.createdAt }
  switch (message.kind) {
    case 'user':
      return {
        ...base,
        kind: 'request',
        state: 'done',
        description: oneLine(message.content),
      }
    case 'agent':
      return {
        ...base,
        kind: 'reply',
        state: 'done',
        description: oneLine(message.content),
      }
    case 'toolCall':
      return {
        ...base,
        kind: 'tool',
        tool: message.tool,
        state: message.isError
          ? 'failed'
          : message.status === 'running'
            ? runActive
              ? 'running'
              : 'pending'
            : 'done',
        description: argsLine(message.args),
      }
    case 'approval':
      return {
        ...base,
        kind: 'approval',
        tool: message.tool,
        state:
          message.status === 'pending'
            ? 'pending'
            : message.status === 'rejected'
              ? 'failed'
              : 'done',
        description: argsLine(message.args),
      }
    case 'error':
      return {
        ...base,
        kind: 'error',
        state: 'failed',
        description: oneLine(message.content),
      }
  }
}

/** Start, each message in order, then the end (or "running") */
export function timelineSteps(detail: RunDetail): TimelineStep[] {
  const { run } = detail
  const active = isActive(run.status)
  const steps: TimelineStep[] = [
    {
      id: 'start',
      at: run.startedAt,
      kind: 'started',
      state: 'done',
      description: oneLine(run.input),
    },
  ]
  // Messages arrive in conversation order
  for (const message of detail.messages) {
    steps.push(messageStep(message, active))
  }
  if (active) {
    steps.push({
      id: 'end',
      at: Date.now(),
      kind: 'running',
      state: run.status === 'awaitingApproval' ? 'pending' : 'running',
      description: '',
      status: run.status,
    })
  } else {
    steps.push({
      id: 'end',
      at: run.endedAt ?? run.startedAt,
      kind: 'finished',
      state: FAILED_STATUSES.includes(run.status) ? 'failed' : 'done',
      description: oneLine(run.summary ?? ''),
      status: run.status,
    })
  }
  return withDurations(steps)
}

/**
 * Each finished step lasts until the next one starts; a step still going
 * (or waiting) has no duration yet, nor does the closing row
 */
function withDurations(steps: TimelineStep[]): TimelineStep[] {
  return steps.map((step, index) => {
    const next = steps[index + 1]
    if (!next || step.state === 'running' || step.state === 'pending') {
      return step
    }
    return { ...step, durationMs: Math.max(0, next.at - step.at) }
  })
}

// ---------------------------------------------------------------------------
// Files

export type FileOp = 'read' | 'write' | 'list' | 'shell' | 'attached'

export interface FileEntry {
  key: string
  op: FileOp
  path: string
  sizeBytes: number | null
  /** Tool call or attachment ID */
  sourceId: string
  isError: boolean
}

const PATH_KEYS = ['path', 'file_path', 'filePath', 'file', 'filename']

function stringArg(args: unknown, keys: string[]): string | null {
  if (!args || typeof args !== 'object' || Array.isArray(args)) return null
  const record = args as Record<string, unknown>
  for (const key of keys) {
    const value = record[key]
    if (typeof value === 'string' && value.length > 0) return value
  }
  return null
}

function byteLength(text: string): number {
  return new TextEncoder().encode(text).length
}

function fileOp(tool: string): FileOp | null {
  // `write_file`, `filesystem__read_file`, `listDir` → words
  const words = tool
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .toLowerCase()
    .split(/[^a-z0-9]+/)
  const has = (...names: string[]) => names.some(n => words.includes(n))
  // A browser screenshot is saved as a file (`browser/screenshot-….png`)
  if (has('write', 'create', 'save', 'edit', 'append', 'screenshot'))
    return 'write'
  if (has('read', 'cat', 'open')) return 'read'
  if (has('list', 'ls', 'dir')) return 'list'
  if (has('shell', 'exec', 'bash', 'command')) return 'shell'
  return null
}

/** The file a browser screenshot was saved to, from its result */
function savedScreenshot(result: string | null | undefined): string | null {
  return /Saved the screenshot as (\S+) in/.exec(result ?? '')?.[1] ?? null
}

/** Files the run touched, from tool call arguments and attachments */
export function filesTouched(detail: RunDetail): FileEntry[] {
  const files: FileEntry[] = []
  for (const message of detail.messages) {
    if (message.kind === 'user') {
      for (const attachment of message.attachments) {
        files.push({
          key: `attachment:${attachment.id}`,
          op: 'attached',
          path: attachment.name,
          sizeBytes: attachment.sizeBytes,
          sourceId: attachment.id,
          isError: false,
        })
      }
      continue
    }
    if (message.kind !== 'toolCall') continue
    const op = fileOp(message.tool)
    if (!op) continue
    const path =
      op === 'shell'
        ? stringArg(message.args, ['cmd', 'command', 'script'])
        : (stringArg(message.args, PATH_KEYS) ??
          savedScreenshot(message.result))
    if (!path) continue
    const content =
      op === 'write'
        ? stringArg(message.args, ['content', 'contents', 'text', 'data'])
        : op === 'read' && !message.isError
          ? message.result
          : null
    files.push({
      key: message.id,
      op,
      path,
      sizeBytes: content === null ? null : byteLength(content),
      sourceId: message.id,
      isError: message.isError,
    })
  }
  return files
}

export function generatedFiles(detail: RunDetail): FileEntry[] {
  const seen = new Set<string>()
  return filesTouched(detail).filter(file => {
    if (file.op !== 'write' || file.isError || seen.has(file.path)) return false
    seen.add(file.path)
    return true
  })
}

// ---------------------------------------------------------------------------
// Output

/** The run's last agent message (or its summary) */
export function finalOutput(detail: RunDetail): string | null {
  for (let i = detail.messages.length - 1; i >= 0; i--) {
    const message = detail.messages[i]
    if (message?.kind === 'agent' && message.content.trim()) {
      return message.content
    }
  }
  return detail.run.summary
}

export interface RunStats {
  durationMs: number | null
  inputTokens: number
  outputTokens: number
  costUsd: number | null
  toolCalls: number
  failedToolCalls: number
  filesWritten: number
  approvals: number
}

export function runStats(detail: RunDetail): RunStats {
  const { run } = detail
  const toolCalls = detail.messages.filter(m => m.kind === 'toolCall')
  return {
    durationMs: run.endedAt === null ? null : run.endedAt - run.startedAt,
    inputTokens: run.inputTokens,
    outputTokens: run.outputTokens,
    costUsd: run.costUsd,
    toolCalls: toolCalls.length,
    failedToolCalls: toolCalls.filter(m => m.isError).length,
    filesWritten: generatedFiles(detail).length,
    approvals: detail.messages.filter(m => m.kind === 'approval').length,
  }
}

// ---------------------------------------------------------------------------
// Numbers in the output

export interface NumericPoint {
  label: string
  value: number
  /** As written, e.g. "$12.4M" or "12%" */
  display: string
}

export interface NumericData {
  /** Chart / card title (a table's value column or a JSON key) */
  title: string | null
  points: NumericPoint[]
}

/** "$1,234.5", "12%", "-3.2" → number; anything else → null */
export function parseNumber(text: string): number | null {
  const cleaned = text
    .trim()
    .replace(/^[*_`]+|[*_`]+$/g, '')
    .replace(/[$€£¥%,\s]/g, '')
    .replace(/^\+/, '')
  if (!/^-?\d+(\.\d+)?$/.test(cleaned)) return null
  const value = Number(cleaned)
  return Number.isFinite(value) ? value : null
}

const MAX_POINTS = 24

function fromJson(value: unknown): NumericData | null {
  if (Array.isArray(value)) {
    const points = value
      .map((v, i) =>
        typeof v === 'number' && Number.isFinite(v)
          ? { label: String(i + 1), value: v, display: String(v) }
          : null
      )
      .filter((p): p is NumericPoint => p !== null)
    return points.length >= 2 && points.length === value.length
      ? { title: null, points: points.slice(0, MAX_POINTS) }
      : null
  }
  if (!value || typeof value !== 'object') return null
  const entries = Object.entries(value as Record<string, unknown>)
  const numeric = entries.filter(
    (e): e is [string, number] =>
      typeof e[1] === 'number' && Number.isFinite(e[1])
  )
  if (numeric.length >= 2) {
    return {
      title: null,
      points: numeric.slice(0, MAX_POINTS).map(([label, v]) => ({
        label,
        value: v,
        display: String(v),
      })),
    }
  }
  // `{"revenue": [1, 2, 3]}`
  for (const [key, inner] of entries) {
    const series = Array.isArray(inner) ? fromJson(inner) : null
    if (series) return { ...series, title: key }
  }
  return null
}

function jsonCandidates(text: string): string[] {
  const candidates: string[] = []
  for (const match of text.matchAll(/```(?:json)?\s*\n([\s\S]*?)```/g)) {
    if (match[1]) candidates.push(match[1])
  }
  candidates.push(text)
  return candidates
}

function tableCells(line: string): string[] {
  return line
    .trim()
    .replace(/^\|/, '')
    .replace(/\|$/, '')
    .split('|')
    .map(cell => cell.trim())
}

function fromTable(text: string): NumericData | null {
  const lines = text.split('\n')
  for (let i = 0; i + 2 < lines.length; i++) {
    const header = lines[i]
    const divider = lines[i + 1]
    if (!header?.includes('|') || !divider) continue
    if (!/^\s*\|?\s*:?-{3,}:?\s*(\|\s*:?-{3,}:?\s*)*\|?\s*$/.test(divider)) {
      continue
    }
    const headers = tableCells(header)
    const rows: string[][] = []
    for (let j = i + 2; j < lines.length; j++) {
      const line = lines[j]
      if (!line?.includes('|')) break
      rows.push(tableCells(line))
    }
    if (rows.length === 0) continue
    // The first column whose every cell is a number (after the label column)
    for (let col = 1; col < headers.length; col++) {
      const values = rows.map(row => parseNumber(row[col] ?? ''))
      if (values.every(v => v !== null)) {
        return {
          title: headers[col] ?? null,
          points: rows.slice(0, MAX_POINTS).map((row, r) => ({
            label: row[0] ?? String(r + 1),
            value: values[r] as number,
            display: row[col] ?? '',
          })),
        }
      }
    }
  }
  return null
}

/**
 * Numbers the output states unambiguously: a JSON object/array of numbers
 * (bare or fenced) or a markdown table with a numeric column. Prose numbers
 * are ignored on purpose; guessing would invent KPIs.
 */
export function detectNumericData(text: string | null): NumericData | null {
  if (!text) return null
  for (const candidate of jsonCandidates(text)) {
    try {
      const data = fromJson(JSON.parse(candidate.trim()))
      if (data) return data
    } catch {
      // Not JSON
    }
  }
  return fromTable(text)
}

// ---------------------------------------------------------------------------
// Formatting

export function formatBytes(bytes: number, locale: string): string {
  const units = ['B', 'KB', 'MB', 'GB']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  return `${value.toLocaleString(locale, { maximumFractionDigits: unit === 0 ? 0 : 1 })} ${units[unit]}`
}

/** `services::run_analysis::NO_API_KEY` */
export const NO_API_KEY_ERROR = 'Add an API key in Settings to use run analysis'

const CITATION_GROUP =
  /\[((?:\s*(?:event|message|artifact|revision):[^\],;\s]+\s*[,;]?)+)\]/gi

/** `[event:12, message:m1]` → inline code, so citations read as tags */
export function formatCitations(markdown: string): string {
  return markdown.replace(
    CITATION_GROUP,
    (_, inner: string) => `\`${inner.trim()}\``
  )
}
