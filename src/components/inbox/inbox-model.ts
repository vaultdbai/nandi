/**
 * The Inbox: everything that waits on the user right now, from the sources
 * that exist today (work-model §2.3, `who = me`). Pure, so the order and
 * the badge count are tested without rendering. Home will read the same
 * model once it exists (task 7); a backend `list_inbox` can replace it later.
 */
import type {
  GatewayAssignment,
  GatewayHold,
  PendingApproval,
  PendingClarification,
  PendingManualStep,
  RequestReturn,
  TrustRequest,
  WorkItem,
} from '@/lib/bindings'

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

/** The nav badge: what waits on the user now (failures aren't counted) */
/** Offered to the user, not waiting on them: kept out of the badge */
function isOffered(item: InboxItem): boolean {
  return item.kind === 'failed' || item.kind === 'requestReturn'
}

export function inboxBadgeCount(items: InboxItem[]): number {
  return items.filter(item => !isOffered(item)).length
}
