import { formatRelativeTime } from '@/lib/format-time'
import type {
  ChiefOfStaffDigest,
  DigestItem,
  DigestKind,
  IntakeAction,
  QueuedWork,
  RequestSource,
  Verdict,
  WaitingStep,
} from '@/lib/bindings'
import type { View } from '@/store/navigation-store'

/** Header order of the summary line */
const KINDS: DigestKind[] = ['needsYou', 'started', 'updated', 'finished']

export interface DigestRow {
  requestId: string
  title: string
  kind: DigestKind
  /** i18n key of the badge: the kind, or the end status for `finished` */
  labelKey: string
  /** i18n key of where it came from */
  sourceKey: string
  /** The news, for `updated` rows only */
  text: string | null
  when: string
  /** Its Chief of Staff thread, when it came through intake */
  threadId: string | null
  /** The step waiting on the user: it gets "Mark done" */
  waitingStep: WaitingStep | null
}

export interface DigestSummaryPart {
  /** `queued` is the "waiting for your OK" count, always first */
  kind: DigestKind | 'queued'
  count: number
}

export interface QueueRow {
  id: string
  /**
   * `start` begins a Request; `update` adds news to `requestTitle`; `run`
   * runs a teammate's request on `requestTitle` (agent · capability)
   */
  action: IntakeAction
  requestId: string | null
  /** The Request the news would join, or the agent a teammate asked */
  requestTitle: string | null
  text: string
  sender: string | null
  sourceKey: string
  verdict: Verdict
  reasons: string[]
  when: string
  /** Blocked items can only be dismissed */
  canApprove: boolean
  /** Suspicious items ask before they start */
  needsConfirm: boolean
  /** Its Chief of Staff conversation (`intake:<id>`) */
  threadId: string
  /** What the plan has someone do, in order (`start` items) */
  planSteps: string[]
}

/** "Since you were away", or the plain title when there is no last look */
export function digestTitleKey(digest: ChiefOfStaffDigest): string {
  return digest.since === null
    ? 'home.chiefOfStaffDigest.titleAlways'
    : 'home.chiefOfStaffDigest.title'
}

export const SOURCE_KEYS: Record<RequestSource, string> = {
  me: 'home.chiefOfStaffDigest.source.me',
  email: 'home.chiefOfStaffDigest.source.email',
  calendar: 'home.chiefOfStaffDigest.source.calendar',
  chiefOfStaff: 'home.chiefOfStaffDigest.source.chiefOfStaff',
  teammate: 'home.chiefOfStaffDigest.source.teammate',
}

export function digestLabelKey(item: DigestItem): string {
  if (item.kind !== 'finished')
    return `home.chiefOfStaffDigest.kind.${item.kind}`
  if (item.status === 'failed' || item.status === 'cancelled') {
    return `home.chiefOfStaffDigest.finished.${item.status}`
  }
  return 'home.chiefOfStaffDigest.finished.done'
}

export function digestRows(
  digest: ChiefOfStaffDigest,
  locale: string,
  now: Date = new Date()
): DigestRow[] {
  return digest.items.map(item => ({
    requestId: item.requestId,
    title: item.title,
    kind: item.kind,
    labelKey: digestLabelKey(item),
    sourceKey: SOURCE_KEYS[item.source],
    text: item.kind === 'updated' ? item.text : null,
    when: formatRelativeTime(item.at, locale, now),
    threadId: item.threadId,
    waitingStep: item.waitingStep,
  }))
}

/** Where a row opens: its thread when it has one, else the Request */
export function rowView(row: {
  threadId: string | null
  requestId: string
}): View {
  return row.threadId
    ? { kind: 'intake-thread', conversationId: row.threadId }
    : { kind: 'request', requestId: row.requestId }
}

/** One queued item as a row */
export function queueRow(
  work: QueuedWork,
  locale: string,
  now: Date = new Date()
): QueueRow {
  return {
    id: work.id,
    action: work.action,
    requestId: work.requestId,
    requestTitle: work.requestTitle,
    text: work.text,
    sender: work.sender,
    sourceKey: SOURCE_KEYS[work.source],
    verdict: work.screening.verdict,
    reasons: work.screening.verdict === 'clean' ? [] : work.screening.reasons,
    when: formatRelativeTime(work.at, locale, now),
    canApprove: work.screening.verdict !== 'blocked',
    needsConfirm: work.screening.verdict === 'suspicious',
    threadId: work.threadId,
    planSteps: work.plan?.steps.map(step => step.description) ?? [],
  }
}

/** The screened items waiting for the user, oldest first */
export function queueRows(
  digest: ChiefOfStaffDigest,
  locale: string,
  now: Date = new Date()
): QueueRow[] {
  return digest.queue.map(work => queueRow(work, locale, now))
}

/** "Waiting for your OK" first, then counts per kind in header order, without the empty ones */
export function digestSummary(digest: ChiefOfStaffDigest): DigestSummaryPart[] {
  const kinds: DigestSummaryPart[] = KINDS.map(kind => ({
    kind,
    count: digest.items.filter(item => item.kind === kind).length,
  }))
  return [
    { kind: 'queued' as const, count: digest.queue.length },
    ...kinds,
  ].filter(part => part.count > 0)
}
