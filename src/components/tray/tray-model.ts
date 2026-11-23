/**
 * The work tray: one glance at everything moving, on every page. Three
 * groups, newest-first where it matters: Needs you (the Inbox), Working
 * (Home's Running) and Done (Home's Recent). Each row is one short line
 * with a colour for its state and a link; each group has "All". Pure: the
 * tray passes in Home's feed and renders what comes back.
 *
 * A to-do is one line too: its text, a due chip and a marker coloured by
 * kind and urgency (`todo-model`). Handed to an agent, it is Working ("<agent>
 * is on it") in place of that run's own row; done today, it is Done.
 */
import type { NavLabel } from '@/components/layout/nav-tree'
import type { ActivityStatus } from '@/components/home/activity-model'
import type { HomeRow } from '@/components/home/home-model'
import {
  assignmentItem,
  firstLine,
  inboxItemView,
  inboxRowLabel,
  waitingOnYou,
  type InboxItem,
} from '@/components/inbox/inbox-model'
import {
  doneSince,
  dueChip,
  isTodo,
  reminderRings,
  startOfDay,
  todoTone,
  workingTodos,
  type TodoTone,
} from '@/components/inbox/todo-model'
import type { GatewayAssignment } from '@/lib/bindings'
import type { View } from '@/store/navigation-store'

/**
 * Rows Working and Done show before their "All" link. Needs you shows
 * every item it counts (`waitingOnYou`), so its number is its rows.
 */
export const TRAY_LIMITS = { working: 6, done: 8 } as const

export type TrayGroupId = 'needsYou' | 'working' | 'done'

/** The row's colour: waiting on you, moving, finished well, or not */
export type TrayTone = 'needsYou' | 'working' | 'done' | 'failed' | 'quiet'

export interface TrayRow {
  id: string
  label: NavLabel
  tone: TrayTone
  /** Where the row goes */
  view: View
  /** A short note on the right: step n of m, the agent */
  meta: NavLabel | null
  /** A tool approval the tray can decide in place */
  approval: { agentId: string; approvalId: string } | null
  /** A to-do's marker: its tone, and a bell for a reminder */
  todo?: { tone: TodoTone; bell: 'none' | 'quiet' | 'ringing' }
}

export interface TrayGroup {
  id: TrayGroupId
  /** Every item in the group, shown or not */
  total: number
  rows: TrayRow[]
  /** Where "All" goes */
  all: View
}

const INBOX: View = { kind: 'inbox' }

const key = (k: string, values?: Record<string, string | number>): NavLabel =>
  values ? { key: k, values } : { key: k }

const STATUS_TONES: Record<ActivityStatus, TrayTone> = {
  running: 'working',
  waiting: 'needsYou',
  done: 'done',
  failed: 'failed',
  cancelled: 'quiet',
}

/**
 * One Inbox item as a line, opening its Ask chat (`inboxItemView`); a tool
 * approval can also be decided in place
 */
export function inboxTrayRow(item: InboxItem, now = 0): TrayRow {
  const row = (label: NavLabel, tone: TrayTone = 'needsYou'): TrayRow => ({
    id: item.id,
    label,
    tone,
    view: inboxItemView(item),
    meta: null,
    approval: null,
  })
  switch (item.kind) {
    case 'noModel':
      return row(key('tray.noModel'))
    case 'approval':
      return {
        ...row(key('tray.approval', { tool: item.approval.tool })),
        meta: { text: item.agentName },
        approval: {
          agentId: item.approval.agentId,
          approvalId: item.approval.id,
        },
      }
    case 'workflowApproval':
      return row(key('tray.workflowApproval', { title: item.item.title }))
    case 'gatewayHold':
      return row(key('tray.hold', { name: item.hold.callerName }))
    case 'assignment':
      return isTodo(item.assignment)
        ? todoTrayRow(item.assignment, now)
        : row(key('tray.assignment', { name: item.assignment.callerName }))
    case 'clarification':
      return row({ text: item.clarification.question.question })
    case 'manualStep':
      return row(key('tray.manualStep', { workflow: item.step.workflowName }))
    case 'trustRequest':
      return row(
        key('tray.trustRequest', {
          name:
            item.request.memberName ??
            item.request.handle ??
            item.request.instanceId,
        })
      )
    case 'failed':
      return row({ text: item.item.title }, 'failed')
    case 'requestReturn':
      return row(key('tray.requestReturn', { name: item.ret.fromName }), 'done')
    case 'intake':
      return row(key('tray.intake', { text: firstLine(item.work.text) }))
    case 'knowMe':
      return row(inboxRowLabel(item), 'quiet')
    case 'retro':
      return row(
        inboxRowLabel(item),
        item.retro.status === 'pending' ? 'needsYou' : 'quiet'
      )
  }
}

/** The due chip as a short label (`tray.due.<key>`) */
function chipLabel(due: number | null, now: number): NavLabel | null {
  const chip = dueChip(due, now)
  if (!chip) return null
  const { key: k, ...values } = chip
  return key(`tray.due.${k}`, values)
}

/**
 * A to-do waiting on the user as one line: its first line, the due chip,
 * and its marker; it opens its Ask chat
 */
export function todoTrayRow(todo: GatewayAssignment, now: number): TrayRow {
  return {
    id: `assignment-${todo.taskId}`,
    label: { text: firstLine(todo.request) },
    tone: 'needsYou',
    view: inboxItemView(assignmentItem(todo)),
    meta: chipLabel(todo.dueAt, now),
    approval: null,
    todo: {
      tone: todoTone(todo, now),
      bell:
        todo.kind !== 'reminder'
          ? 'none'
          : reminderRings(todo, now)
            ? 'ringing'
            : 'quiet',
    },
  }
}

/** A to-do an agent works on: "<agent> is on it", opening its chat */
function workingTodoRow(
  todo: GatewayAssignment,
  agentName: (id: string) => string
): TrayRow {
  const agent = todo.assignedAgent ?? ''
  return {
    id: `todo-working-${todo.taskId}`,
    label: { text: firstLine(todo.request) },
    tone: 'working',
    view: { kind: 'agent', agentId: agent, tab: 'chat' },
    meta: key('tray.todoWorking', { agent: agentName(agent) }),
    approval: null,
  }
}

/** A to-do done today, quiet */
function doneTodoRow(todo: GatewayAssignment): TrayRow {
  return {
    id: `todo-done-${todo.taskId}`,
    label: { text: firstLine(todo.request) },
    tone: 'done',
    view: INBOX,
    meta: key('tray.todoDone'),
    approval: null,
  }
}

/** One Running or Recent row as a line */
export function homeTrayRow(row: HomeRow): TrayRow {
  return {
    id: row.id,
    label: { text: row.title },
    tone: STATUS_TONES[row.status],
    view: row.view,
    meta: row.progress
      ? key('tray.step', {
          done: Math.min(row.progress.done + 1, row.progress.total),
          total: row.progress.total,
        })
      : row.agent
        ? { text: row.agent }
        : null,
    approval: null,
  }
}

/**
 * The three groups, in tray order. Needs you keeps the Inbox's urgency
 * order; Working and Done keep Home's.
 */
export function buildTray(input: {
  inbox: readonly InboxItem[]
  running: readonly HomeRow[]
  recent: readonly HomeRow[]
  /** The user's to-dos (`list_todos`) */
  todos?: readonly GatewayAssignment[]
  now?: number
  agentName?: (id: string) => string
}): TrayGroup[] {
  const now = input.now ?? 0
  const todos = input.todos ?? []
  const agentName = input.agentName ?? ((id: string) => id)
  const needsYou = waitingOnYou(input.inbox, now)
  const working = workingTodos(todos)
  // A handed to-do stands for its run
  const runs = new Set(working.map(todo => `run:${todo.runId}`))
  const running = [
    ...working.map(todo => workingTodoRow(todo, agentName)),
    ...input.running.filter(row => !runs.has(row.id)).map(homeTrayRow),
  ]
  const done = [
    ...doneSince(todos, startOfDay(now)).map(doneTodoRow),
    ...input.recent.map(homeTrayRow),
  ]
  return [
    {
      id: 'needsYou',
      total: needsYou.length,
      rows: needsYou.map(item => inboxTrayRow(item, now)),
      all: INBOX,
    },
    {
      id: 'working',
      total: running.length,
      rows: running.slice(0, TRAY_LIMITS.working),
      all: { kind: 'home-activity', filters: { status: 'running' } },
    },
    {
      id: 'done',
      total: done.length,
      rows: done.slice(0, TRAY_LIMITS.done),
      all: { kind: 'home-activity' },
    },
  ]
}
