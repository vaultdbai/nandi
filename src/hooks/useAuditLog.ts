import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import {
  unwrapResult,
  type AuditEntry,
  type JsonValue,
} from '@/lib/tauri-bindings'
import type { AuditLogTab } from '@/store/navigation-store'

/**
 * Paged entries from one audit log, redacted again on this side.
 *
 * No polling: run and trigger events invalidate `auditQueryKeys.all` (see
 * `agent-sync`); egress and pilot lines have no event, so the view offers
 * a refresh.
 */

export type AuditKind = AuditLogTab

export const AUDIT_KINDS: readonly AuditKind[] = [
  'llm',
  'egress',
  'triggers',
  'pilots',
  'connectors',
]

/** Entries per page (the server pages) */
export const AUDIT_PAGE_SIZE = 100

/** The most entries the server returns per request (`MAX_AUDIT_PAGE`) */
const MAX_SERVER_PAGE = 500

/** Raw-log downloads stop here */
export const MAX_DOWNLOAD_ENTRIES = 50_000

export const REDACTED = '[redacted]'

export const auditQueryKeys = {
  all: ['auditLog'] as const,
  kind: (kind: AuditKind) => ['auditLog', kind] as const,
  page: (kind: AuditKind, limit: number, offset: number) =>
    ['auditLog', kind, limit, offset] as const,
}

/** Field names whose (non-numeric) values are secrets */
export const SECRET_KEY_PATTERN = /password|token|secret|key/i

/** Secrets inside free text, with what replaces them */
const SECRET_TEXT_PATTERNS: [RegExp, string][] = [
  [/\b(bearer|basic)\s+[\w.~+/=-]+/gi, `$1 ${REDACTED}`],
  [
    /\b(password|passwd|secret|token|[\w-]*key)(\s*[=:]\s*"?)[^\s"&,;]+/gi,
    `$1$2${REDACTED}`,
  ],
  [/\bsk-[\w-]{16,}/g, REDACTED],
  [/\b(?:AKIA|ASIA)[0-9A-Z]{16}\b/g, REDACTED],
  [/\bgh[pousr]_[A-Za-z0-9]{20,}/g, REDACTED],
]

export function redactText(text: string): string {
  return SECRET_TEXT_PATTERNS.reduce(
    (current, [pattern, replacement]) => current.replace(pattern, replacement),
    text
  )
}

/**
 * Replaces secrets: the values of fields named like
 * `password|token|secret|key` (counts such as `inputTokens` stay), and
 * secret-looking text anywhere.
 */
export function redactValue(value: JsonValue): JsonValue {
  if (typeof value === 'string') return redactText(value)
  if (Array.isArray(value)) return value.map(redactValue)
  if (value === null || typeof value !== 'object') return value
  const redacted: Record<string, JsonValue> = {}
  for (const [key, field] of Object.entries(value)) {
    if (field === undefined) continue
    const secret =
      SECRET_KEY_PATTERN.test(key) &&
      field !== null &&
      typeof field !== 'number' &&
      typeof field !== 'boolean'
    redacted[key] = secret ? REDACTED : redactValue(field)
  }
  return redacted
}

export function redactEntry(entry: AuditEntry): AuditEntry {
  return {
    ...entry,
    agentId: entry.agentId === null ? null : redactText(entry.agentId),
    tool: entry.tool === null ? null : redactText(entry.tool),
    outcome: redactText(entry.outcome),
    argsRedacted:
      entry.argsRedacted === null ? null : redactValue(entry.argsRedacted),
  }
}

/** One page (0-based) of the `kind` log, newest first, redacted. */
export function useAuditLog(
  kind: AuditKind,
  page = 0,
  pageSize = AUDIT_PAGE_SIZE
) {
  const offset = page * pageSize
  return useQuery({
    queryKey: auditQueryKeys.page(kind, pageSize, offset),
    queryFn: async () =>
      unwrapResult(await client.listAuditLog(kind, pageSize, offset)).map(
        redactEntry
      ),
    placeholderData: keepPreviousData,
  })
}

/** Every entry of the `kind` log (up to `max`), newest first, redacted. */
export async function fetchAuditLog(
  kind: AuditKind,
  max = MAX_DOWNLOAD_ENTRIES
): Promise<AuditEntry[]> {
  const entries: AuditEntry[] = []
  while (entries.length < max) {
    const limit = Math.min(MAX_SERVER_PAGE, max - entries.length)
    const page = unwrapResult(
      await client.listAuditLog(kind, limit, entries.length)
    )
    entries.push(...page.map(redactEntry))
    if (page.length < limit) break
  }
  return entries
}

/** One JSON object per line, oldest first (the log's own order) */
export function entriesToJsonl(entries: AuditEntry[]): string {
  return [...entries]
    .reverse()
    .map(entry => JSON.stringify(entry))
    .join('\n')
    .concat(entries.length > 0 ? '\n' : '')
}

/** The columns a search can be narrowed to */
export type AuditColumn = 'all' | 'agent' | 'tool' | 'outcome' | 'details'

/** Case-insensitive match of `search` against one column (or all) */
export function matchesAuditSearch(
  entry: AuditEntry,
  search: string,
  column: AuditColumn = 'all'
): boolean {
  const needle = search.trim().toLowerCase()
  if (!needle) return true
  const columns: Record<Exclude<AuditColumn, 'all'>, string> = {
    agent: entry.agentId ?? '',
    tool: entry.tool ?? '',
    outcome: entry.outcome,
    details:
      entry.argsRedacted === null ? '' : JSON.stringify(entry.argsRedacted),
  }
  const haystacks =
    column === 'all' ? Object.values(columns) : [columns[column]]
  return haystacks.some(text => text.toLowerCase().includes(needle))
}
