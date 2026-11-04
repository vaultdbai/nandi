/**
 * Pure rules for Run Detail's header and Logs: a readable line per event,
 * the kinds the filter offers, what a CLI runtime decided on its own, and
 * whether stopping or re-running could repeat something outward. No React.
 */
import type {
  ChatMessage,
  JsonValue,
  RunDetail,
  RunEventRecord,
} from '@/lib/bindings'
import { oneLine } from './run-detail-model'

// ---------------------------------------------------------------------------
// Title

/** The input's first line, cut at 60 characters */
export function inputHeadline(input: string, max = 60): string {
  const first = input.trim().split('\n')[0]?.trim() ?? ''
  return first.length > max ? `${first.slice(0, max - 1)}…` : first
}

// ---------------------------------------------------------------------------
// Logs

export const LOG_KINDS = [
  'model',
  'tool',
  'approval',
  'error',
  'other',
] as const
export type LogKind = (typeof LOG_KINDS)[number]

export interface LogEntry {
  id: string
  at: number
  kind: LogKind
  /** i18n key under `runDetail.logs.line.*` */
  line: string
  /** Values for that key */
  values: Record<string, string>
  /** The raw event (`kind` and `payload`), for the JSON toggle */
  raw: RunEventRecord
}

type Payload = Record<string, JsonValue | undefined>

const record = (value: JsonValue | undefined): Payload =>
  value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Payload)
    : {}

const text = (value: JsonValue | undefined): string =>
  typeof value === 'string' ? value : ''

/** The tool a message names, with its most telling argument */
function toolLine(message: Payload): Record<string, string> {
  const args = record(message.args)
  const detail = ['cmd', 'command', 'path', 'file_path', 'query', 'url']
    .map(key => text(args[key]))
    .find(Boolean)
  return { tool: text(message.tool), detail: oneLine(detail ?? '', 80) }
}

/** What one recorded event says, in a line (unknown kinds keep their name) */
export function logEntry(event: RunEventRecord): LogEntry {
  const payload = record(event.payload)
  const base = { id: String(event.id), at: event.createdAt, raw: event }
  const entry = (
    kind: LogKind,
    line: string,
    values: Record<string, string> = {}
  ): LogEntry => ({ ...base, kind, line, values })

  switch (event.kind) {
    case 'agent://run': {
      const status = text(record(payload.run).status)
      return entry(status === 'failed' ? 'error' : 'other', 'run', { status })
    }
    case 'agent://message': {
      const message = record(payload.message)
      switch (text(message.kind)) {
        case 'user':
          return entry('other', 'request', {
            text: oneLine(text(message.content), 80),
          })
        case 'agent':
          return entry('model', 'reply', {
            text: oneLine(text(message.content), 80),
          })
        case 'error':
          return entry('error', 'error', {
            text: oneLine(text(message.content), 120),
          })
        case 'toolCall':
          return entry('tool', 'toolCall', toolLine(message))
        case 'approval':
          return entry('approval', 'approvalRequest', toolLine(message))
        default:
          return entry('other', 'message')
      }
    }
    case 'agent://token':
      return entry('model', 'token')
    case 'agent://tool_call':
      return entry('tool', 'toolCall', toolLine(record(payload.message)))
    case 'agent://tool_result':
      return payload.isError === true
        ? entry('error', 'toolFailed', {
            text: oneLine(text(payload.result), 120),
          })
        : entry('tool', 'toolResult', {
            text: oneLine(text(payload.result), 80),
          })
    case 'agent://approval_request':
      return entry(
        'approval',
        'approvalRequest',
        toolLine(record(payload.message))
      )
    case 'agent://approval_resolved':
      return entry('approval', 'approvalResolved', {
        status: text(payload.status),
      })
    case 'agent://turn_done': {
      const status = text(payload.status)
      return entry(status === 'failed' ? 'error' : 'other', 'turnDone', {
        status,
      })
    }
    case 'agent://turn_aborted':
      return entry('error', 'turnAborted', {
        reason: oneLine(text(payload.reason), 120),
      })
    case 'agent://run_compacted':
      return entry('other', 'compacted')
    case 'agent://tools_loaded':
      return entry('other', 'toolsLoaded', {
        count: String(Array.isArray(payload.tools) ? payload.tools.length : 0),
      })
    case 'harness://permission_request':
      return entry('approval', 'permission', {
        title: oneLine(text(payload.title), 80),
        decision: text(payload.decision),
      })
    default:
      return entry('other', 'unknown', { kind: event.kind })
  }
}

export interface LogFilter {
  /** `null`: every kind */
  kind: LogKind | null
  text: string
}

/** Entries of the chosen kind whose line or raw event contains the text */
export function filterLogs(
  entries: LogEntry[],
  filter: LogFilter,
  lineOf: (entry: LogEntry) => string
): LogEntry[] {
  const needle = filter.text.trim().toLowerCase()
  return entries.filter(entry => {
    if (filter.kind && entry.kind !== filter.kind) return false
    if (!needle) return true
    return (
      lineOf(entry).toLowerCase().includes(needle) ||
      JSON.stringify(entry.raw.payload).toLowerCase().includes(needle)
    )
  })
}

// ---------------------------------------------------------------------------
// CLI runtimes

export interface RuntimeDecision {
  id: string
  at: number
  /** What the runtime was about to do, in its own words */
  title: string
  allowed: boolean
}

/** What a CLI runtime allowed or denied on its own during the run */
export function runtimeDecisions(detail: RunDetail): RuntimeDecision[] {
  return detail.events
    .filter(event => event.kind === 'harness://permission_request')
    .map(event => {
      const payload = record(event.payload)
      return {
        id: String(event.id),
        at: event.createdAt,
        title: text(payload.title),
        allowed: text(payload.decision).startsWith('allow'),
      }
    })
}

/**
 * The run went through a CLI runtime (Claude Code, Codex, Gemini CLI): its
 * tool calls carry the harness prefix, or it recorded the runtime's own
 * permission decisions. Nandi doesn't gate those runs' tools.
 */
export function ranOnCliRuntime(detail: RunDetail): boolean {
  return (
    detail.messages.some(
      m => m.kind === 'toolCall' && m.id.startsWith('harness-')
    ) || detail.events.some(e => e.kind === 'harness://permission_request')
  )
}

// ---------------------------------------------------------------------------
// Side effects

/** Tool names that change files, send something, or hand work on */
const OUTWARD_TOOL =
  /(^|[^a-z])(write|edit|create|delete|remove|save|append|move|rename|send|post|put|patch|email|mail|publish|push|upload|deploy|shell|bash|exec|execute|command|run|delegate)($|[^a-z])/i

/** Whether a tool can change things outside the run */
export function isOutwardTool(tool: string): boolean {
  return OUTWARD_TOOL.test(tool.replace(/([a-z])([A-Z])/g, '$1_$2'))
}

const outwardCall = (message: ChatMessage) =>
  (message.kind === 'toolCall' || message.kind === 'approval') &&
  isOutwardTool(message.tool)

/** Re-running repeats calls that wrote, sent or delegated: ask first */
export function rerunNeedsConfirm(detail: RunDetail): boolean {
  return detail.messages.some(outwardCall)
}

/** Stopping mid-way leaves outward work half done, or drops an approval */
export function stopNeedsConfirm(detail: RunDetail): boolean {
  return detail.messages.some(
    m => outwardCall(m) || (m.kind === 'approval' && m.status === 'pending')
  )
}
