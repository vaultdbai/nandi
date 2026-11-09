/**
 * What Home lists under Running and Recent, and its tiles, from work items
 * (workflow runs, delegated tasks) and activity rows (agent runs, incoming
 * delegations). Pure; `useHomeFeed` feeds it, and the companion reuses
 * both. Inbox items (approvals, manual steps) aren't repeated here.
 */
import type { WorkItem, WorkState } from '@/lib/bindings'
import { workItemView } from '@/lib/work'
import type { View } from '@/store/navigation-store'
import type { ActivityRow, ActivityStatus } from './activity-model'

/** Recent covers this long */
export const RECENT_WINDOW_MS = 24 * 60 * 60 * 1000
export const MAX_RECENT = 8
/** A running row with no update for this long is flagged */
export const STALE_AFTER_MS = 10 * 60 * 1000

/** Who or what a row came from, for its chip */
export type HomeSource =
  | { kind: 'you' | 'trigger' | 'schedule' | 'workflow' }
  | { kind: 'to' | 'from'; name: string }

export type HomeRetry =
  | { kind: 'request'; requestId: string }
  | { kind: 'workflow'; workflowId: string }

export interface HomeRow {
  id: string
  title: string
  status: ActivityStatus
  source: HomeSource
  /** The agent doing the work, by name */
  agent: string | null
  /** "step n of m", when known */
  progress: { done: number; total: number } | null
  startedAt: number
  endedAt: number | null
  /** When it last changed (stale detection) */
  updatedAt: number
  view: View
  /** Delegations I sent: rate once done */
  rating: { workId: string; value: number | null } | null
  retry: HomeRetry | null
}

const WORK_STATUS: Record<WorkState, ActivityStatus> = {
  active: 'running',
  waiting: 'waiting',
  done: 'done',
  failed: 'failed',
  cancelled: 'cancelled',
}

const ENDED = new Set<ActivityStatus>(['done', 'failed', 'cancelled'])

/** In the Inbox already (it needs the user), so not repeated */
function inInbox(item: WorkItem): boolean {
  const kind = item.nextAction?.kind
  return kind === 'approval' || kind === 'manualStep'
}

export function workRow(item: WorkItem): HomeRow {
  const status = WORK_STATUS[item.state]
  const ended = ENDED.has(status)
  return {
    id: `work:${item.id}`,
    title: item.title,
    status,
    source:
      item.kind === 'task'
        ? { kind: 'to', name: item.owner ?? item.title }
        : item.kind === 'trigger'
          ? { kind: 'trigger' }
          : item.kind === 'request'
            ? { kind: 'you' }
            : { kind: 'workflow' },
    agent: null,
    progress:
      item.progress.totalSteps > 0
        ? {
            done: item.progress.completedSteps,
            total: item.progress.totalSteps,
          }
        : null,
    startedAt: item.startedAt,
    endedAt: ended ? item.updatedAt : null,
    updatedAt: item.updatedAt,
    view: workItemView(item),
    rating:
      item.kind === 'task' && status === 'done'
        ? { workId: item.id, value: item.rating }
        : null,
    retry:
      status !== 'failed' || item.kind === 'request'
        ? null
        : item.kind === 'task'
          ? { kind: 'request', requestId: item.id }
          : item.workflowId
            ? { kind: 'workflow', workflowId: item.workflowId }
            : null,
  }
}

export function activityHomeRow(row: ActivityRow): HomeRow {
  return {
    id: row.id,
    title: row.title,
    status: row.status,
    source:
      row.direction === 'incoming'
        ? { kind: 'from', name: row.member ?? '' }
        : row.direction === 'outgoing'
          ? { kind: 'to', name: row.member ?? '' }
          : { kind: row.source === 'peer' ? 'you' : row.source },
    agent: row.agent,
    progress: null,
    startedAt: row.startedAt,
    endedAt: row.endedAt,
    updatedAt: row.endedAt ?? row.startedAt,
    view: row.view,
    rating: null,
    retry:
      row.status === 'failed' && row.direction === 'outgoing'
        ? { kind: 'request', requestId: row.id.replace(/^task:/, '') }
        : null,
  }
}

export interface HomeFeed {
  running: HomeRow[]
  recent: HomeRow[]
}

/**
 * Running (oldest first, nothing waiting on the user) and Recent (ended in
 * the last day, newest first, at most 8). Work items win over the activity
 * row of the same task; workflow-started runs show as their workflow.
 */
export function buildHomeFeed(input: {
  work: WorkItem[]
  activity: ActivityRow[]
  now: number
}): HomeFeed {
  const { work, activity, now } = input
  const taskIds = new Set(
    work.filter(item => item.kind === 'task').map(item => `task:${item.id}`)
  )
  const rows = [
    ...work
      .filter(item => item.kind !== 'manualStep' && !inInbox(item))
      .map(workRow),
    ...activity
      .filter(row => !taskIds.has(row.id) && row.source !== 'workflow')
      // Outgoing delegations come from work items (with progress, rating)
      .filter(row => row.direction !== 'outgoing')
      // Waiting runs wait on an approval: that's in the Inbox
      .filter(row => row.status !== 'waiting')
      .map(activityHomeRow),
  ]
  const running = rows
    .filter(row => !ENDED.has(row.status))
    .sort((a, b) => a.startedAt - b.startedAt)
  const recent = rows
    .filter(
      row =>
        ENDED.has(row.status) &&
        (row.endedAt ?? row.startedAt) >= now - RECENT_WINDOW_MS
    )
    .sort((a, b) => (b.endedAt ?? b.startedAt) - (a.endedAt ?? a.startedAt))
    .slice(0, MAX_RECENT)
  return { running, recent }
}

/** A running row that hasn't moved in a while */
export function isStale(row: HomeRow, now: number): boolean {
  return !ENDED.has(row.status) && now - row.updatedAt > STALE_AFTER_MS
}
