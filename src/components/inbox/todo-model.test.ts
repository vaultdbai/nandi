import { describe, expect, it } from 'vitest'
import type { GatewayAssignment } from '@/lib/bindings'
import {
  buildInbox,
  inboxBadgeCount,
  waitingOnYou,
  type InboxSources,
} from './inbox-model'
import {
  countsInBadge,
  daysUntil,
  doneSince,
  dueChip,
  dueText,
  isDayDue,
  groupTodos,
  relatedLink,
  reminderRings,
  TODO_TONE_TEXT,
  todoState,
  todoTone,
  workingTodos,
} from './todo-model'

const HOUR = 60 * 60 * 1000
const DAY = 24 * HOUR
// Noon, local time, so "today" and "tomorrow" don't depend on the clock
const NOW = new Date(2030, 0, 15, 12, 0, 0).getTime()

const todo = (
  overrides: Partial<GatewayAssignment> = {}
): GatewayAssignment => ({
  taskId: 'todo-1',
  callerInstance: '',
  callerName: 'Nandi',
  request: 'Reply to the landlord about the deposit',
  status: 'open',
  note: null,
  createdAt: NOW - DAY,
  dueAt: null,
  closedAt: null,
  links: [],
  kind: 'do',
  source: 'email',
  sourceRef: 'msg-1',
  relatedRef: null,
  runId: null,
  assignedAgent: null,
  runStatus: null,
  ...overrides,
})

describe('todoState: open, handed, the run ended, done', () => {
  it('follows the hand-off through its run', () => {
    expect(todoState(todo())).toBe('open')
    const handed = todo({ runId: 'r1', assignedAgent: 'writer' })
    expect(todoState({ ...handed, runStatus: 'running' })).toBe('working')
    expect(todoState({ ...handed, runStatus: 'queued' })).toBe('working')
    expect(todoState({ ...handed, runStatus: 'awaitingApproval' })).toBe(
      'working'
    )
    // The run ended: back to the user, "mark done?"
    expect(todoState({ ...handed, runStatus: 'completed' })).toBe('finished')
    expect(todoState({ ...handed, runStatus: 'failed' })).toBe('finished')
    expect(todoState({ ...handed, runStatus: null })).toBe('finished')
    expect(todoState(todo({ status: 'done', runId: 'r1' }))).toBe('done')
  })

  it('takes a working to-do out of the Inbox into Working', () => {
    const working = todo({
      taskId: 'todo-w',
      runId: 'r1',
      assignedAgent: 'writer',
      runStatus: 'running',
    })
    const finished = {
      ...working,
      taskId: 'todo-f',
      runStatus: 'completed' as const,
    }
    const sources: InboxSources = {
      now: NOW,
      noModel: false,
      approvals: [],
      clarifications: [],
      manualSteps: [],
      holds: [],
      assignments: [working, finished, todo()],
      trustRequests: [],
      work: [],
      dismissed: [],
      returns: [],
      queue: [],
      agentName: id => id,
    }
    const ids = buildInbox(sources).map(item => item.id)
    expect(ids).toEqual(['assignment-todo-f', 'assignment-todo-1'])
    expect(workingTodos([working, finished]).map(t => t.taskId)).toEqual([
      'todo-w',
    ])
  })
})

describe('todoTone and the colour coding', () => {
  it('colours by urgency first, then by kind', () => {
    expect(todoTone(todo({ dueAt: NOW - HOUR }), NOW)).toBe('overdue')
    expect(todoTone(todo({ dueAt: NOW + DAY }), NOW)).toBe('soon')
    expect(todoTone(todo({ dueAt: NOW + 2 * DAY }), NOW)).toBe('soon')
    expect(todoTone(todo({ dueAt: NOW + 5 * DAY }), NOW)).toBe('do')
    expect(todoTone(todo(), NOW)).toBe('do')
    expect(todoTone(todo({ kind: 'check' }), NOW)).toBe('check')
    expect(
      todoTone(todo({ kind: 'reminder', dueAt: NOW + 20 * DAY }), NOW)
    ).toBe('reminder')
    // A reminder near its date takes the urgency tones
    expect(todoTone(todo({ kind: 'reminder', dueAt: NOW + DAY }), NOW)).toBe(
      'soon'
    )
    expect(todoTone(todo({ kind: 'reminder', dueAt: NOW - DAY }), NOW)).toBe(
      'overdue'
    )
  })

  it('uses theme tokens only, a reminder its own accent', () => {
    expect(TODO_TONE_TEXT.overdue).toBe('text-destructive')
    expect(TODO_TONE_TEXT.soon).toBe('text-warning')
    expect(TODO_TONE_TEXT.do).toBe('text-primary')
    expect(TODO_TONE_TEXT.check).toBe('text-info')
    expect(TODO_TONE_TEXT.reminder).toBe('text-highlight')
  })

  it('rings a reminder within 3 days of its date or past it', () => {
    const reminder = (dueAt: number | null) => todo({ kind: 'reminder', dueAt })
    expect(reminderRings(reminder(NOW + 20 * DAY), NOW)).toBe(false)
    expect(reminderRings(reminder(NOW + 3 * DAY), NOW)).toBe(true)
    expect(reminderRings(reminder(NOW - DAY), NOW)).toBe(true)
    expect(reminderRings(reminder(null), NOW)).toBe(true)
    expect(reminderRings(todo({ dueAt: NOW }), NOW)).toBe(false)
  })
})

describe('what the Home badge counts', () => {
  it('counts open to-dos and ringing reminders, never far ones or working ones', () => {
    const far = todo({ taskId: 'far', kind: 'reminder', dueAt: NOW + 20 * DAY })
    const near = todo({ taskId: 'near', kind: 'reminder', dueAt: NOW + DAY })
    const late = todo({ taskId: 'late', kind: 'reminder', dueAt: NOW - DAY })
    const check = todo({ taskId: 'check', kind: 'check' })
    const doIt = todo({ taskId: 'do', dueAt: NOW + 30 * DAY })
    const finished = todo({
      taskId: 'finished',
      runId: 'r1',
      runStatus: 'completed',
    })
    // Counted
    for (const t of [near, late, check, doIt, finished]) {
      expect(countsInBadge(t, NOW), t.taskId).toBe(true)
    }
    // Shown, not counted
    expect(countsInBadge(far, NOW)).toBe(false)
    const working = todo({ runId: 'r2', runStatus: 'running' })
    expect(countsInBadge(working, NOW)).toBe(false)
    expect(countsInBadge(todo({ status: 'done' }), NOW)).toBe(false)

    const items = buildInbox({
      now: NOW,
      noModel: false,
      approvals: [],
      clarifications: [],
      manualSteps: [],
      holds: [],
      assignments: [far, near, check],
      trustRequests: [],
      work: [],
      dismissed: [],
      returns: [],
      queue: [],
      agentName: id => id,
    })
    // The far reminder is listed, sorted low by its date, but not counted
    expect(items.map(item => item.id)).toEqual([
      'assignment-near',
      'assignment-far',
      'assignment-check',
    ])
    expect(inboxBadgeCount(items, NOW)).toBe(2)
  })

  it('is one rule for the badge, the Inbox header and the tray', () => {
    const far = todo({ taskId: 'far', kind: 'reminder', dueAt: NOW + 20 * DAY })
    const checks = [
      todo({ taskId: 'c1', kind: 'check' }),
      todo({ taskId: 'c2', kind: 'check' }),
    ]
    const items = buildInbox({
      now: NOW,
      noModel: false,
      approvals: [],
      clarifications: [],
      manualSteps: [],
      holds: [],
      assignments: [far, ...checks],
      trustRequests: [],
      work: [],
      dismissed: [],
      returns: [],
      queue: [],
      knowMe: { todoOpen: true, recheck: false, shownAt: NOW },
      agentName: id => id,
    })
    // Listed in the Inbox: both checks, the far reminder, know-me
    expect(items).toHaveLength(4)
    // Waiting on you (and counted everywhere): the two checks
    const waiting = waitingOnYou(items, NOW).map(item => item.id)
    expect(waiting).toEqual(['assignment-c1', 'assignment-c2'])
    expect(inboxBadgeCount(items, NOW)).toBe(waiting.length)
  })
})

/** The end of the day `days` from NOW's, as a date-only due is stored */
const endOfDay = (days: number) =>
  new Date(2030, 0, 15 + days, 23, 59, 59).getTime()

describe('date-only dues read by the day', () => {
  it('knows a day from a time', () => {
    expect(isDayDue(endOfDay(0))).toBe(true)
    expect(isDayDue(NOW + DAY)).toBe(false)
    expect(daysUntil(endOfDay(3), NOW)).toBe(3)
    expect(daysUntil(endOfDay(-2), NOW)).toBe(-2)
  })

  it('says Due today, tomorrow, a date, yesterday or how many days late', () => {
    expect(dueText(null, NOW)).toBeNull()
    expect(dueText(endOfDay(0), NOW)).toEqual({ key: 'dueToday' })
    expect(dueText(endOfDay(1), NOW)).toEqual({ key: 'dueTomorrow' })
    expect(dueText(endOfDay(10), NOW)).toEqual({
      key: 'dueOn',
      on: endOfDay(10),
    })
    expect(dueText(endOfDay(-1), NOW)).toEqual({ key: 'overdueYesterday' })
    expect(dueText(endOfDay(-2), NOW)).toEqual({ key: 'overdueDays', days: 2 })
  })

  it('keeps hours and minutes for a due with a time', () => {
    expect(dueText(NOW + 5 * HOUR, NOW)).toEqual({
      key: 'dueIn',
      duration: 5 * HOUR,
    })
    expect(dueText(NOW - 30 * HOUR, NOW)).toEqual({
      key: 'overdueBy',
      duration: 30 * HOUR,
    })
    expect(dueText(NOW + 3 * DAY, NOW)).toEqual({
      key: 'dueOn',
      on: NOW + 3 * DAY,
    })
  })

  it('gives the tray chip by the day too', () => {
    expect(dueChip(endOfDay(0), NOW)).toEqual({ key: 'today' })
    expect(dueChip(endOfDay(1), NOW)).toEqual({ key: 'tomorrow' })
    expect(dueChip(endOfDay(5), NOW)).toEqual({ key: 'inDays', days: 5 })
    expect(dueChip(endOfDay(-2), NOW)).toEqual({ key: 'overdue', days: 2 })
    expect(dueChip(endOfDay(-1), NOW)).toEqual({ key: 'overdue', days: 1 })
  })
})

describe('dueChip with a time', () => {
  it('says hours today, tomorrow, in days, or how overdue', () => {
    expect(dueChip(null, NOW)).toBeNull()
    expect(dueChip(NOW + 2 * HOUR, NOW)).toEqual({ key: 'inHours', hours: 2 })
    expect(dueChip(NOW + 8 * HOUR, NOW)).toEqual({ key: 'inHours', hours: 8 })
    expect(dueChip(NOW + DAY, NOW)).toEqual({ key: 'tomorrow' })
    expect(dueChip(NOW + 4 * DAY, NOW)).toEqual({ key: 'inDays', days: 4 })
    expect(dueChip(NOW - 3 * HOUR, NOW)).toEqual({
      key: 'overdueHours',
      hours: 3,
    })
    expect(dueChip(NOW - 3 * DAY, NOW)).toEqual({ key: 'overdue', days: 3 })
  })
})

describe('groupTodos', () => {
  it('groups Do now, Check and close, Reminders, each by due date', () => {
    const groups = groupTodos([
      todo({ taskId: 'r-far', kind: 'reminder', dueAt: NOW + 30 * DAY }),
      todo({ taskId: 'd-undated' }),
      todo({ taskId: 'd-late', dueAt: NOW - DAY }),
      todo({ taskId: 'c', kind: 'check' }),
      todo({ taskId: 'd-soon', dueAt: NOW + DAY }),
      todo({ taskId: 'r-near', kind: 'reminder', dueAt: NOW + DAY }),
      todo({ taskId: 'gone', status: 'done' }),
      // Work a person assigned isn't a to-do
      todo({ taskId: 'theirs', source: null }),
    ])
    expect(groups.map(g => [g.id, g.todos.map(t => t.taskId)])).toEqual([
      ['doNow', ['d-late', 'd-soon', 'd-undated']],
      ['check', ['c']],
      ['reminders', ['r-near', 'r-far']],
    ])
    expect(groupTodos([todo({ kind: 'check' })]).map(g => g.id)).toEqual([
      'check',
    ])
  })

  it('lists done ones since a time, the latest first', () => {
    const done = [
      todo({ taskId: 'old', status: 'done', closedAt: NOW - 3 * DAY }),
      todo({ taskId: 'a', status: 'done', closedAt: NOW - HOUR }),
      todo({ taskId: 'b', status: 'done', closedAt: NOW - 2 * HOUR }),
      todo({ taskId: 'open' }),
    ]
    expect(doneSince(done, NOW - DAY).map(t => t.taskId)).toEqual(['a', 'b'])
  })
})

describe('relatedLink', () => {
  it('opens what Nandi prepared, by its thread', () => {
    expect(relatedLink(null)).toBeNull()
    expect(relatedLink('inbox:approval:a1')).toEqual({
      label: 'queued',
      view: { kind: 'inbox-thread', conversationId: 'inbox:approval:a1' },
    })
    expect(relatedLink('intake:q1')).toEqual({
      label: 'queued',
      view: { kind: 'intake-thread', conversationId: 'intake:q1' },
    })
    expect(relatedLink('inbox:requestReturn:t1')?.label).toBe('prepared')
    expect(relatedLink('req-9')).toEqual({
      label: 'prepared',
      view: { kind: 'request', requestId: 'req-9' },
    })
  })
})
