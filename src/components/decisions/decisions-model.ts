/**
 * The Decisions page's model: the filters, how a decision reads, and which
 * view a decision's chat opens. Pure, so it is tested without rendering.
 */
import type { DecisionFilter, DecisionRecord } from '@/lib/bindings'
import type { View } from '@/store/navigation-store'

/**
 * What a decision is made on (`services::inbox::audit`): the Inbox kinds,
 * a call a policy refused, and a connector grant
 */
export const DECISION_KINDS = [
  'approval',
  'workflowApproval',
  'gatewayHold',
  'assignment',
  'clarification',
  'manualStep',
  'trustRequest',
  'failed',
  'requestReturn',
  'intake',
  'task',
  'policy',
  'connectorGrant',
] as const

/** The words a decision starts with; a choice can follow ("approved (once)") */
export const DECISION_WORDS = [
  'approved',
  'allowed',
  'refused',
  'declined',
  'answered',
  'done',
  'dismissed',
  'retried',
  'reassigned',
  'trusted',
  'not now',
  'blocked',
  'granted',
  'removed',
] as const

/** An empty field means "any" */
export interface DecisionFilters {
  kind: string
  decision: string
  search: string
}

export const NO_FILTERS: DecisionFilters = {
  kind: '',
  decision: '',
  search: '',
}

export function isFiltered(filters: DecisionFilters): boolean {
  return Boolean(filters.kind || filters.decision || filters.search.trim())
}

/** What the backend takes (`null`: no filter at all) */
export function toFilter(filters: DecisionFilters): DecisionFilter | null {
  if (!isFiltered(filters)) return null
  const search = filters.search.trim()
  return {
    kind: filters.kind || null,
    decision: filters.decision || null,
    search: search || null,
  }
}

export type DecisionTone = 'positive' | 'negative' | 'neutral'

const POSITIVE = new Set(['approved', 'allowed', 'answered', 'done', 'trusted'])
const NEGATIVE = new Set(['refused', 'declined', 'blocked', 'not now'])

/** Whether a decision said yes, no, or something else (dismissed, retried) */
export function decisionTone(decision: string): DecisionTone {
  const word = decision.trim().toLowerCase()
  const first = word.split(/[\s(]/)[0] ?? ''
  if (POSITIVE.has(word) || POSITIVE.has(first)) return 'positive'
  if (NEGATIVE.has(word) || NEGATIVE.has(first)) return 'negative'
  return 'neutral'
}

/** The view a decision's chat opens, if the chat is still there */
export function chatView(record: DecisionRecord): View | null {
  const id = record.chatId
  if (!id || !record.chatExists) return null
  if (id.startsWith('intake:')) {
    return { kind: 'intake-thread', conversationId: id }
  }
  if (id.startsWith('inbox:')) {
    return { kind: 'inbox-thread', conversationId: id }
  }
  return null
}

/** "November 2026", in the reader's language */
export function monthLabel(month: string, locale: string): string {
  const [year, number] = month.split('-').map(Number)
  if (!year || !number) return month
  return new Intl.DateTimeFormat(locale, {
    year: 'numeric',
    month: 'long',
    timeZone: 'UTC',
  }).format(new Date(Date.UTC(year, number - 1, 1)))
}

/** "Nov 8, 2026, 2:32 PM", in the reader's language and time zone */
export function decisionTime(at: number, locale: string): string {
  return new Intl.DateTimeFormat(locale, {
    dateStyle: 'medium',
    timeStyle: 'short',
  }).format(new Date(at))
}

/** The i18n key part of a known decision word (`not now` -> `notNow`) */
export function decisionWordKey(word: string): string {
  return word.replace(/ (\w)/g, (_, c: string) => c.toUpperCase())
}

/**
 * A decision as its known word plus the detail written after it:
 * `approved (once)` -> `approved`, `(once)`; a decision that starts with
 * no known word has no word, only the detail (its full text)
 */
export function splitDecision(decision: string): {
  word: (typeof DECISION_WORDS)[number] | null
  rest: string
} {
  const text = decision.trim()
  const lower = text.toLowerCase()
  const word = [...DECISION_WORDS]
    .sort((a, b) => b.length - a.length)
    .find(w => {
      if (!lower.startsWith(w)) return false
      const next = lower.charAt(w.length)
      return next === '' || next === ' ' || next === '('
    })
  return word
    ? { word, rest: text.slice(word.length).trim() }
    : { word: null, rest: text }
}
