/**
 * The Inbox: everything that waits on the user right now, from the sources
 * that exist today (work-model §2.3, `who = me`). Pure, so the order and
 * the badge count are tested without rendering. Home will read the same
 * model once it exists (task 7); a backend `list_inbox` can replace it later.
 *
 * Every item is an Ask chat with the Chief of Staff (`services::inbox`):
 * `inboxThreadId` names it and `inboxItemView` opens it.
 */
import type {
  KnowMeStatus,
  GatewayAssignment,
  GatewayHold,
  PendingApproval,
  PendingClarification,
  PendingManualStep,
  QueuedWork,
  RequestReturn,
  TrustRequest,
  WorkItem,
} from '@/lib/bindings'
import type { NavLabel } from '@/components/layout/nav-tree'
import { workItemView } from '@/lib/work'
import type { View } from '@/store/navigation-store'
import { approvalSummary } from './approval-model'

/** Failures stay offered this long */
const FAILED_WINDOW_MS = 24 * 60 * 60 * 1000

interface Base {
  /** Stable across refreshes (React key, dismissals) */
  id: string
  /** When it started waiting (epoch ms) */
  createdAt: number
  /** When it stops waiting on its own (epoch ms), if ever */
  deadline: number | null
}

export type InboxItem =
  | (Base & { kind: 'noModel' })
  | (Base & {
      kind: 'approval'
      approval: PendingApproval
      agentName: string
    })
  | (Base & { kind: 'workflowApproval'; item: WorkItem })
  /** Another Nandi's request held for the owner (task 5) */
  | (Base & { kind: 'gatewayHold'; hold: GatewayHold })
  /** Work another Nandi assigned to the owner, open until Done or Decline */
  | (Base & { kind: 'assignment'; assignment: GatewayAssignment })
  | (Base & { kind: 'clarification'; clarification: PendingClarification })
  | (Base & { kind: 'manualStep'; step: PendingManualStep })
  /** Someone refused because this Nandi doesn't trust them yet (task 12) */
  | (Base & { kind: 'trustRequest'; request: TrustRequest })
  | (Base & { kind: 'failed'; item: WorkItem })
  /** A request the user sent came back while nobody watched */
  | (Base & { kind: 'requestReturn'; ret: RequestReturn })
  /** Work from mail, the calendar or a teammate, waiting for the user's OK */
  | (Base & { kind: 'intake'; work: QueuedWork })
  /**
   * Getting to know you (`services::knowme`): never "done", never counted;
   * `recheck` once the person answered before ("is this still right?")
   */
  | (Base & { kind: 'knowMe'; recheck: boolean })

export interface InboxSources {
  now: number
  noModel: boolean
  approvals: PendingApproval[]
  clarifications: PendingClarification[]
  manualSteps: PendingManualStep[]
  /** Requests from other Nandis waiting for the owner's approval */
  holds: GatewayHold[]
  /** Work assigned to the owner that's still open */
  assignments: GatewayAssignment[]
  /** People waiting to be trusted (`list_trust_requests`) */
  trustRequests: TrustRequest[]
  work: WorkItem[]
  /** IDs dismissed from the Inbox */
  dismissed: string[]
  /** Requests that came back while nobody watched (`list_request_returns`) */
  returns: RequestReturn[]
  /** The intake queue: work from outside waiting for the user's OK */
  queue: QueuedWork[]
  /** The know-me to-do (`get_know_me`), when loaded */
  knowMe?: Pick<KnowMeStatus, 'todoOpen' | 'recheck' | 'shownAt'> | null
  agentName: (agentId: string) => string
}

/** Soonest deadline first, then the oldest; no deadline after any deadline */
function byUrgency(a: InboxItem, b: InboxItem): number {
  if (a.deadline !== null && b.deadline !== null) {
    return a.deadline - b.deadline || a.createdAt - b.createdAt
  }
  if (a.deadline !== null) return -1
  if (b.deadline !== null) return 1
  return a.createdAt - b.createdAt
}

export function buildInbox(sources: InboxSources): InboxItem[] {
  const { now } = sources
  const items: InboxItem[] = []

  if (sources.noModel) {
    items.push({
      kind: 'noModel',
      id: 'no-model',
      createdAt: 0,
      deadline: null,
    })
  }

  for (const approval of sources.approvals) {
    // Past its deadline: it's expiring (the sweep closes it within a minute)
    if (approval.expiresAt !== null && approval.expiresAt <= now) continue
    items.push({
      kind: 'approval',
      id: `approval-${approval.id}`,
      approval,
      agentName: sources.agentName(approval.agentId),
      createdAt: approval.createdAt,
      deadline: approval.expiresAt,
    })
  }

  // A run waiting on a manual step shows as the step instead
  const manualRuns = new Set(sources.manualSteps.map(step => step.runId))
  for (const item of sources.work) {
    if (
      item.state === 'waiting' &&
      item.nextAction?.kind === 'approval' &&
      !manualRuns.has(item.id)
    ) {
      items.push({
        kind: 'workflowApproval',
        id: `work-${item.id}`,
        item,
        createdAt: item.updatedAt,
        deadline: item.nextAction.deadline,
      })
    }
  }

  // Only questions escalated to a person; the agent answers the rest
  for (const clarification of sources.clarifications) {
    if (clarification.approvalId === null) continue
    if (clarification.deadline !== null && clarification.deadline <= now) {
      continue
    }
    items.push({
      kind: 'clarification',
      id: `clarification-${clarification.taskId}-${clarification.question.id}`,
      clarification,
      createdAt: now,
      deadline: clarification.deadline,
    })
  }

  for (const hold of sources.holds) {
    // Past its expiry: the gateway closes it
    if (hold.status !== 'pending' || hold.expiresAt <= now) continue
    items.push({
      kind: 'gatewayHold',
      id: `hold-${hold.taskId}`,
      hold,
      createdAt: hold.createdAt,
      deadline: hold.expiresAt,
    })
  }

  // Overdue ones stay: they're the most urgent
  for (const assignment of sources.assignments) {
    if (assignment.status !== 'open') continue
    items.push({
      kind: 'assignment',
      id: `assignment-${assignment.taskId}`,
      assignment,
      createdAt: assignment.createdAt,
      deadline: assignment.dueAt,
    })
  }

  for (const request of sources.trustRequests) {
    items.push({
      kind: 'trustRequest',
      id: `trust-${request.instanceId}`,
      request,
      createdAt: request.firstAt,
      deadline: null,
    })
  }

  for (const step of sources.manualSteps) {
    items.push({
      kind: 'manualStep',
      id: `manual-${step.runId}-${step.stepId}`,
      step,
      createdAt: step.readyAt,
      deadline: step.deadline,
    })
  }

  for (const ret of sources.returns) {
    items.push({
      kind: 'requestReturn',
      id: `return-${ret.taskId}`,
      ret,
      createdAt: ret.returnedAt,
      deadline: null,
    })
  }

  // One line per intake thread: news for the same work joins it
  const threads = new Set<string>()
  for (const work of sources.queue) {
    if (work.status !== 'pending' || threads.has(work.threadId)) continue
    threads.add(work.threadId)
    items.push({
      kind: 'intake',
      id: `intake-${work.threadId}`,
      work,
      createdAt: work.at,
      deadline: null,
    })
  }

  if (sources.knowMe?.todoOpen) {
    items.push({
      kind: 'knowMe',
      id: 'know-me',
      recheck: sources.knowMe.recheck,
      createdAt: sources.knowMe.shownAt ?? now,
      deadline: null,
    })
  }

  const dismissed = new Set(sources.dismissed)
  // A request that came back failed is already a return: not twice
  const returned = new Set(sources.returns.map(ret => ret.taskId))
  for (const item of sources.work) {
    if (
      item.state === 'failed' &&
      !dismissed.has(item.id) &&
      !returned.has(item.id) &&
      item.updatedAt >= now - FAILED_WINDOW_MS
    ) {
      items.push({
        kind: 'failed',
        id: `failed-${item.id}`,
        item,
        createdAt: item.updatedAt,
        deadline: null,
      })
    }
  }

  // Returns and failures are offered, not waiting: they go after what needs
  // a decision
  const waiting = items.filter(item => !isOffered(item)).sort(byUrgency)
  const rest = items.filter(isOffered).sort(byUrgency)
  return [...waiting, ...rest]
}

/**
 * Offered to the user, not waiting on them: kept out of the badge (a
 * know-me question never nags)
 */
function isOffered(item: InboxItem): boolean {
  return (
    item.kind === 'failed' ||
    item.kind === 'requestReturn' ||
    item.kind === 'knowMe'
  )
}

/** The nav badge: what waits on the user now (failures aren't counted) */

export function inboxBadgeCount(items: InboxItem[]): number {
  return items.filter(item => !isOffered(item)).length
}

/** An ID inside a thread's ID: `/` and `%` escaped (as `inbox::item` does) */
function part(id: string): string {
  return id.replace(/%/g, '%25').replace(/\//g, '%2F')
}

/**
 * The item's Ask chat with the Chief of Staff (`inbox:<kind>:<id>`, an
 * intake item's own `intake:<id>`), or `null` when it has none: a missing
 * model is fixed in Settings, and a workflow approval that doesn't name its
 * step is reviewed on its run
 */
export function inboxThreadId(item: InboxItem): string | null {
  const thread = (id: string) => `inbox:${item.kind}:${id}`
  switch (item.kind) {
    case 'noModel':
      return null
    case 'approval':
      return thread(part(item.approval.id))
    case 'workflowApproval': {
      const target = item.item.nextAction?.target
      if (!target?.runId || !target.stepId) return null
      return thread(`${part(target.runId)}/${part(target.stepId)}`)
    }
    case 'gatewayHold':
      return thread(part(item.hold.taskId))
    case 'assignment':
      return thread(part(item.assignment.taskId))
    case 'clarification':
      return thread(
        `${part(item.clarification.taskId)}/${part(item.clarification.question.id)}`
      )
    case 'manualStep':
      return thread(`${part(item.step.runId)}/${part(item.step.stepId)}`)
    case 'trustRequest':
      return thread(part(item.request.instanceId))
    case 'requestReturn':
      return thread(part(item.ret.taskId))
    case 'failed':
      return thread(part(item.item.id))
    case 'intake':
      return item.work.threadId
    case 'knowMe':
      return 'inbox:knowMe:you'
  }
}

/** Where opening the item goes: its chat, else where it's fixed */
export function inboxItemView(item: InboxItem): View {
  if (item.kind === 'intake') {
    return { kind: 'intake-thread', conversationId: item.work.threadId }
  }
  const conversationId = inboxThreadId(item)
  if (conversationId) return { kind: 'inbox-thread', conversationId }
  if (item.kind === 'workflowApproval') return workItemView(item.item)
  return { kind: 'settings', section: 'models' }
}

const label = (key: string, values?: Record<string, string>): NavLabel =>
  values ? { key, values } : { key }

/** The first line of a text, for a one-line row */
export function firstLine(text: string): string {
  return text.trim().split('\n')[0] ?? ''
}

/** The item's one line in the Inbox list */
export function inboxRowLabel(item: InboxItem): NavLabel {
  switch (item.kind) {
    case 'noModel':
      return label('inbox.noModel')
    case 'approval': {
      const summary = approvalSummary(item.approval.tool, item.approval.args)
      return label(`approval.summary.${summary.key}`, summary.values)
    }
    case 'workflowApproval':
      return label('inbox.workflowApproval', { workflow: item.item.title })
    case 'gatewayHold': {
      const { callerName: member, capability } = item.hold
      if (capability === 'assign-me') {
        return label('inbox.hold.titleAssign', { member })
      }
      if (capability === 'ask-me')
        return label('inbox.hold.titleAsk', { member })
      return capability
        ? label('inbox.hold.titleCapability', { member, capability })
        : label('inbox.hold.title', { member })
    }
    case 'assignment':
      return label('inbox.assignment.title', {
        member: item.assignment.callerName,
      })
    case 'clarification':
      return label('inbox.question', {
        question: item.clarification.question.question,
      })
    case 'manualStep':
      return label('work.manual.label', {
        step: item.step.stepId,
        workflow: item.step.workflowName,
      })
    case 'trustRequest':
      return label('inbox.trust.title', {
        name:
          item.request.memberName ??
          item.request.handle ??
          item.request.instanceId,
      })
    case 'failed':
      return label('inbox.failed', { title: item.item.title })
    case 'requestReturn':
      return label(`inbox.return.${item.ret.outcome}`, {
        name: item.ret.fromName,
      })
    case 'intake':
      return label('inbox.intake', { text: firstLine(item.work.text) })
    case 'knowMe':
      return label(item.recheck ? 'inbox.knowMe.recheck' : 'inbox.knowMe.first')
  }
}
