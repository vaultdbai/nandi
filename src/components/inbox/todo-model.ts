/**
 * The user's to-dos: assignments opened here for the user (`source` set,
 * `gateway::todos`). Pure, so the grouping, the urgency colours, the due
 * chip and what the badge counts are tested without rendering. The Inbox
 * and the work tray read them through here.
 */
import type {
  GatewayAssignment,
  RunStatus,
  TodoKind,
  TodoSource,
} from '@/lib/bindings'
import type { View } from '@/store/navigation-store'

const HOUR_MS = 60 * 60 * 1000
const DAY_MS = 24 * HOUR_MS

/** Due this soon (or sooner) reads as urgent */
export const SOON_MS = 2 * DAY_MS
/** A reminder this close to its date rings, and counts in the badge */
export const REMINDER_RINGS_MS = 3 * DAY_MS

/** A to-do: an assignment with a source (work a person assigned has none) */
export function isTodo(assignment: GatewayAssignment): boolean {
  return assignment.source !== null
}

const ACTIVE_RUNS: readonly RunStatus[] = [
  'queued',
  'running',
  'awaitingApproval',
  'compacted',
]

/**
 * Where a to-do stands: waiting on the user, handed to an agent whose run
 * goes (`working`), handed and the run ended (`finished`: the user checks
 * and marks it done), or done
 */
export type TodoState = 'open' | 'working' | 'finished' | 'done'

export function todoState(todo: GatewayAssignment): TodoState {
  if (todo.status !== 'open') return 'done'
  if (todo.runId === null) return 'open'
  // A run we can't see any more (deleted) has ended
  if (todo.runStatus !== null && ACTIVE_RUNS.includes(todo.runStatus)) {
    return 'working'
  }
  return 'finished'
}

/**
 * The colour of a to-do: overdue, due within 2 days, or its kind's own.
 * A reminder keeps its own look until it rings (within 3 days or past its
 * date), then takes the urgency tones like any to-do.
 */
export type TodoTone = 'overdue' | 'soon' | 'do' | 'check' | 'reminder'

export function todoTone(todo: GatewayAssignment, now: number): TodoTone {
  const due = todo.dueAt
  if (due !== null && due <= now) return 'overdue'
  if (due !== null && due - now <= SOON_MS) return 'soon'
  return todo.kind
}

/** Each tone's colour (theme tokens only): a reminder has its own accent */
export const TODO_TONE_TEXT: Record<TodoTone, string> = {
  overdue: 'text-destructive',
  soon: 'text-warning',
  do: 'text-primary',
  check: 'text-info',
  reminder: 'text-highlight',
}

export const TODO_TONE_DOT: Record<TodoTone, string> = {
  overdue: 'bg-destructive',
  soon: 'bg-warning',
  do: 'bg-primary',
  check: 'bg-info',
  reminder: 'bg-highlight',
}

/** Whether a reminder is close enough to its date (or past it) to ring */
export function reminderRings(todo: GatewayAssignment, now: number): boolean {
  return (
    todo.kind === 'reminder' &&
    (todo.dueAt === null || todo.dueAt - now <= REMINDER_RINGS_MS)
  )
}

/**
 * Whether the to-do counts in the Home badge: an open one waiting on the
 * user, except a reminder that doesn't ring yet (the list still shows it)
 */
export function countsInBadge(todo: GatewayAssignment, now: number): boolean {
  const state = todoState(todo)
  if (state === 'working' || state === 'done') return false
  return todo.kind !== 'reminder' || reminderRings(todo, now)
}

/** The small due chip: "today", "2d", "overdue 3d" */
export type DueChip =
  | { key: 'today' }
  | { key: 'tomorrow' }
  | { key: 'inDays'; days: number }
  | { key: 'inHours'; hours: number }
  | { key: 'overdue'; days: number }
  | { key: 'overdueHours'; hours: number }

function sameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  )
}

export function dueChip(due: number | null, now: number): DueChip | null {
  if (due === null) return null
  if (due <= now) {
    const late = now - due
    return late < DAY_MS
      ? { key: 'overdueHours', hours: Math.max(1, Math.floor(late / HOUR_MS)) }
      : { key: 'overdue', days: Math.floor(late / DAY_MS) }
  }
  const at = new Date(due)
  const today = new Date(now)
  if (sameDay(at, today)) {
    return due - now < 6 * HOUR_MS
      ? { key: 'inHours', hours: Math.max(1, Math.ceil((due - now) / HOUR_MS)) }
      : { key: 'today' }
  }
  const tomorrow = new Date(now + DAY_MS)
  if (sameDay(at, tomorrow)) return { key: 'tomorrow' }
  return { key: 'inDays', days: Math.ceil((due - now) / DAY_MS) }
}

/** Home's groups, in order */
export type TodoGroupId = 'doNow' | 'check' | 'reminders'

const GROUP_OF: Record<TodoKind, TodoGroupId> = {
  do: 'doNow',
  check: 'check',
  reminder: 'reminders',
}

export interface TodoGroup {
  id: TodoGroupId
  todos: GatewayAssignment[]
}

/** Soonest due first (overdue on top), undated after, then the oldest */
export function byDue(a: GatewayAssignment, b: GatewayAssignment): number {
  if (a.dueAt !== null && b.dueAt !== null) {
    return a.dueAt - b.dueAt || a.createdAt - b.createdAt
  }
  if (a.dueAt !== null) return -1
  if (b.dueAt !== null) return 1
  return a.createdAt - b.createdAt
}

/**
 * Home's to-do list: open ones (working and finished too) in "Do now",
 * "Check and close" and "Reminders", each by due date; empty groups are
 * left out
 */
export function groupTodos(todos: readonly GatewayAssignment[]): TodoGroup[] {
  const open = todos.filter(todo => isTodo(todo) && todo.status === 'open')
  return (['doNow', 'check', 'reminders'] as const)
    .map(id => ({
      id,
      todos: open.filter(todo => GROUP_OF[todo.kind] === id).sort(byDue),
    }))
    .filter(group => group.todos.length > 0)
}

/** To-dos done since `since` (the start of today for the tray), latest first */
export function doneSince(
  todos: readonly GatewayAssignment[],
  since: number
): GatewayAssignment[] {
  return todos
    .filter(
      todo =>
        isTodo(todo) &&
        todo.status === 'done' &&
        todo.closedAt !== null &&
        todo.closedAt >= since
    )
    .sort((a, b) => (b.closedAt ?? 0) - (a.closedAt ?? 0))
}

/** Open to-dos an agent is working on now (the tray's Working) */
export function workingTodos(
  todos: readonly GatewayAssignment[]
): GatewayAssignment[] {
  return todos.filter(todo => isTodo(todo) && todoState(todo) === 'working')
}

/** The start of the day `now` is in, local time */
export function startOfDay(now: number): number {
  const day = new Date(now)
  day.setHours(0, 0, 0, 0)
  return day.getTime()
}

/** "from an email", "from a chat", … (`inbox.todo.source.<source>`) */
export function sourceKey(source: TodoSource): string {
  return `inbox.todo.source.${source}`
}

/**
 * What Nandi prepared for it, by its thread ID: an approval or incoming
 * work waits for the user's OK; anything else (a Request) is ready to look
 * at. `null` when nothing is linked.
 */
export function relatedLink(
  ref: string | null
): { label: 'queued' | 'prepared'; view: View } | null {
  if (!ref) return null
  if (ref.startsWith('intake:')) {
    return {
      label: 'queued',
      view: { kind: 'intake-thread', conversationId: ref },
    }
  }
  if (ref.startsWith('inbox:')) {
    const queued =
      ref.startsWith('inbox:approval:') ||
      ref.startsWith('inbox:workflowApproval:') ||
      ref.startsWith('inbox:gatewayHold:')
    return {
      label: queued ? 'queued' : 'prepared',
      view: { kind: 'inbox-thread', conversationId: ref },
    }
  }
  return { label: 'prepared', view: { kind: 'request', requestId: ref } }
}
