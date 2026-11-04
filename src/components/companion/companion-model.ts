/**
 * The companion's rules, pure so they're tested without a window: the
 * orb's state and badge, the one-sentence summary, which Inbox items get
 * an OS notification, and how Request links in chat are recognised. The
 * counts come from Home's queries (`useInbox`, `useHomeFeed`), so the orb's
 * badge and the Home nav badge can't disagree.
 */
import type { CompanionBlocker } from '@/lib/bindings'
import type { InboxItem } from '@/components/inbox/inbox-model'
import type { View } from '@/store/navigation-store'

/** Rows the card shows before "+ N more" (companion.md §4.2) */
export const CARD_LIMITS = { needsYou: 3, running: 4, recent: 3 } as const

/** An expiring item is announced this long before its deadline */
export const EXPIRY_WARNING_MS = 5 * 60 * 1000

/** How replies link a Request (`REQUEST_LINK_PREFIX` in studio/prompt.rs) */
export const REQUEST_LINK_PREFIX = 'nandi://request/'

export type OrbState = 'needsYou' | 'running' | 'quiet' | 'problem'

export interface OrbInput {
  /** Home's badge count (`inboxBadgeCount`) */
  inboxCount: number
  running: number
  blocker: CompanionBlocker | null
}

export interface Orb {
  state: OrbState
  /** A number, `!` for a problem, or nothing */
  badge: number | '!' | null
}

/** A problem beats everything; then work waiting on you; then running */
export function orbFor({ inboxCount, running, blocker }: OrbInput): Orb {
  if (blocker !== null) return { state: 'problem', badge: '!' }
  if (inboxCount > 0) return { state: 'needsYou', badge: inboxCount }
  if (running > 0) return { state: 'running', badge: null }
  return { state: 'quiet', badge: null }
}

/**
 * What opening `view` does in the companion window: a Request not pinned
 * yet is pinned here (a live card to watch and ask about), so the card
 * stays useful without the main window; anything else, or the pinned
 * Request opened again (its ›), goes to the main window. Returns the
 * Request to pin, or null for the main window.
 */
export function requestToPin(
  view: View,
  focusedRequestId: string | null
): string | null {
  return view.kind === 'request' && view.requestId !== focusedRequestId
    ? view.requestId
    : null
}

/** How long the bison shows `done` after something that needed you was handled */
export const DONE_POSE_MS = 4000

/**
 * The bison's pose for the orb: waiting while something needs you (or is
 * broken), carrying while agents run, lying down when it's quiet, and
 * `done` for a moment after you handled something
 */
export function poseFor(
  orb: Orb,
  recentlyDone: boolean
): 'rest' | 'wait' | 'carry' | 'done' {
  if (orb.state === 'problem' || orb.state === 'needsYou') return 'wait'
  if (recentlyDone) return 'done'
  return orb.state === 'running' ? 'carry' : 'rest'
}

/** Whether the Inbox count going from `before` to `after` means you just handled one */
export function handledOne(before: number | null, after: number): boolean {
  return before !== null && after < before
}

export interface SummaryCounts {
  needsYou: number
  running: number
  doneToday: number
}

/** The parts of "2 need you · 3 running · 5 done today", zeros left out */
export function summaryParts(
  counts: SummaryCounts
): { key: keyof SummaryCounts; count: number }[] {
  return (['needsYou', 'running', 'doneToday'] as const)
    .map(key => ({ key, count: counts[key] }))
    .filter(part => part.count > 0)
}

/**
 * Inbox items worth interrupting for: approvals, questions, and requests
 * that came back
 */
function interrupts(item: InboxItem): boolean {
  return (
    item.kind === 'approval' ||
    item.kind === 'workflowApproval' ||
    item.kind === 'gatewayHold' ||
    item.kind === 'clarification' ||
    item.kind === 'requestReturn'
  )
}

export type NoticeKind = 'approval' | 'question' | 'expiring' | 'result'

function noticeKind(item: InboxItem): NoticeKind {
  if (item.kind === 'clarification') return 'question'
  if (item.kind === 'requestReturn') return 'result'
  return 'approval'
}

export interface Notice {
  itemId: string
  kind: NoticeKind
  item: InboxItem
}

export interface NoticeState {
  /** Items already seen (announced, or there when the companion started) */
  seen: Set<string>
  /** Items already warned about expiring */
  warned: Set<string>
}

/**
 * What to announce now. Only approvals and questions, only while the
 * companion is hidden or collapsed (`away`), and only once per item; an
 * item 5 minutes from expiring is announced once more. With `state` `null`
 * (the first look), everything present is taken as seen: no burst of
 * notifications at start.
 */
export function noticesFor(input: {
  items: InboxItem[]
  state: NoticeState | null
  now: number
  away: boolean
  enabled: boolean
  /** "Notify me when requests come back" (defaults to `enabled`) */
  results?: boolean
}): { notices: Notice[]; state: NoticeState } {
  const results = input.results ?? input.enabled
  const relevant = input.items.filter(interrupts)
  if (input.state === null) {
    return {
      notices: [],
      state: {
        seen: new Set(relevant.map(item => item.id)),
        warned: new Set(
          relevant
            .filter(item => expiringSoon(item, input.now))
            .map(item => item.id)
        ),
      },
    }
  }
  const seen = new Set(input.state.seen)
  const warned = new Set(input.state.warned)
  const notices: Notice[] = []
  for (const item of relevant) {
    const announce =
      input.away && (item.kind === 'requestReturn' ? results : input.enabled)
    if (!seen.has(item.id)) {
      seen.add(item.id)
      if (announce) {
        notices.push({ itemId: item.id, kind: noticeKind(item), item })
      }
      // A new item that's already close to its deadline: one notice is enough
      if (expiringSoon(item, input.now)) warned.add(item.id)
      continue
    }
    if (!warned.has(item.id) && expiringSoon(item, input.now)) {
      warned.add(item.id)
      if (announce) notices.push({ itemId: item.id, kind: 'expiring', item })
    }
  }
  return { notices, state: { seen, warned } }
}

function expiringSoon(item: InboxItem, now: number): boolean {
  return (
    item.deadline !== null &&
    item.deadline > now &&
    item.deadline - now <= EXPIRY_WARNING_MS
  )
}

/** The Request ID a chat link points at (`nandi://request/<id>`), if any */
export function requestIdFromLink(href: string | undefined): string | null {
  if (!href?.startsWith(REQUEST_LINK_PREFIX)) return null
  const id = decodeURIComponent(href.slice(REQUEST_LINK_PREFIX.length)).trim()
  return id.length > 0 ? id : null
}
