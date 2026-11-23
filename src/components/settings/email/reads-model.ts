/**
 * Settings › Email, each account's files and calendar: what its switches
 * say, what turning one on or off asks for, the calendar line, and the
 * calendar check intervals. Pure.
 */

import type {
  CalendarCheckStatus,
  MailboxStatus,
  MailProvider,
  Reads,
} from '@/lib/bindings'

export type ReadFamily = 'files' | 'calendar'

export const READ_FAMILIES: readonly ReadFamily[] = ['files', 'calendar']

/** Product names: not translated */
export const READ_NAMES: Record<MailProvider, Record<ReadFamily, string>> = {
  outlook: { files: 'OneDrive', calendar: 'Outlook calendar' },
  gmail: { files: 'Google Drive', calendar: 'Google Calendar' },
}

/** What connecting an account asks for: mail, and files and calendar unless the person unticks them */
export const DEFAULT_READS: Reads = { mail: true, files: true, calendar: true }

export interface ReadSwitch {
  family: ReadFamily
  /** "OneDrive", "Google Calendar" */
  name: string
  on: boolean
  /** Read on a sign-in of its own, from before it moved under the account */
  apart: boolean
}

/** The account's files and calendar switches */
export function readSwitches(status: MailboxStatus): ReadSwitch[] {
  return READ_FAMILIES.map(family => ({
    family,
    name: READ_NAMES[status.provider][family],
    on: status.reads[family],
    apart: status.readsApart[family],
  }))
}

/**
 * What turning one read on or off takes. On: a sign-in that asks for it and
 * everything the account already reads (so one sign-in covers them all).
 * Off: just dropping it.
 */
export type ReadChange =
  { kind: 'signIn'; reads: Reads } | { kind: 'drop'; reads: Reads }

export function readChange(
  status: MailboxStatus,
  family: ReadFamily,
  on: boolean
): ReadChange {
  const reads = { ...status.reads, [family]: on }
  return on ? { kind: 'signIn', reads } : { kind: 'drop', reads }
}

/** Reconnect asks again for everything the account reads */
export function reconnectReads(status: MailboxStatus): Reads {
  return { ...status.reads }
}

/** An account without mail (a 0.2 drive or calendar) adds mail by signing in */
export function addMailReads(status: MailboxStatus): Reads {
  return { ...status.reads, mail: true }
}

export type CalendarLine =
  | { key: 'workspace.lastChecked'; when: number; count: number }
  | { key: 'email.reads.calendarNotChecked' }
  | { key: 'email.reads.calendarNeedsSignIn' }
  | { key: 'email.reads.calendarError'; error: string }

/** What the row says about the account's calendar check */
export function calendarLine(
  check: CalendarCheckStatus | null
): CalendarLine | null {
  if (check === null) return null
  if (check.needsSignIn) return { key: 'email.reads.calendarNeedsSignIn' }
  if (check.error !== null)
    return { key: 'email.reads.calendarError', error: check.error }
  if (check.lastCheckedAt === null)
    return { key: 'email.reads.calendarNotChecked' }
  return {
    key: 'workspace.lastChecked',
    when: check.lastCheckedAt,
    count: check.lastFound,
  }
}

/** The calendar check settings matter once an account reads its calendar */
export function anyCalendar(mailboxes: readonly MailboxStatus[]): boolean {
  return mailboxes.some(m => m.reads.calendar)
}

export const CALENDAR_INTERVALS = [15, 30, 60, 120, 240] as const

/** The offered intervals, with the current one if it's set elsewhere */
export function calendarIntervalOptions(current: number): number[] {
  const options: number[] = [...CALENDAR_INTERVALS]
  if (current > 0 && !options.includes(current)) options.push(current)
  return options.sort((a, b) => a - b)
}
