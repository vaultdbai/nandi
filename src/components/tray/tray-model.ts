/**
 * The work tray: one glance at everything moving, on every page. Three
 * groups, newest-first where it matters: Needs you (the Inbox), Working
 * (Home's Running) and Done (Home's Recent). Each row is one short line
 * with a colour for its state and a link; each group has "All". Pure: the
 * tray passes in Home's feed and renders what comes back.
 */
import type { NavLabel } from '@/components/layout/nav-tree'
import type { ActivityStatus } from '@/components/home/activity-model'
import type { HomeRow } from '@/components/home/home-model'
import {
  firstLine,
  inboxItemView,
  type InboxItem,
} from '@/components/inbox/inbox-model'
import type { View } from '@/store/navigation-store'

/** Rows each group shows before its "All" link */
export const TRAY_LIMITS = { needsYou: 5, working: 6, done: 8 } as const

export type TrayGroupId = keyof typeof TRAY_LIMITS

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
export function inboxTrayRow(item: InboxItem): TrayRow {
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
      return row(key('tray.assignment', { name: item.assignment.callerName }))
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
}): TrayGroup[] {
  return [
    {
      id: 'needsYou',
      total: input.inbox.length,
      rows: input.inbox.slice(0, TRAY_LIMITS.needsYou).map(inboxTrayRow),
      all: INBOX,
    },
    {
      id: 'working',
      total: input.running.length,
      rows: input.running.slice(0, TRAY_LIMITS.working).map(homeTrayRow),
      all: { kind: 'home-activity', filters: { status: 'running' } },
    },
    {
      id: 'done',
      total: input.recent.length,
      rows: input.recent.slice(0, TRAY_LIMITS.done).map(homeTrayRow),
      all: { kind: 'home-activity' },
    },
  ]
}
