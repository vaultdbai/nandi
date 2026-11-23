import { describe, expect, it } from 'vitest'
import type { MailboxStatus } from '@/lib/bindings'
import {
  addMailReads,
  anyCalendar,
  calendarIntervalOptions,
  calendarLine,
  DEFAULT_READS,
  readChange,
  readSwitches,
  reconnectReads,
} from './reads-model'

const NONE = { mail: false, files: false, calendar: false }

function account(overrides: Partial<MailboxStatus> = {}): MailboxStatus {
  return {
    provider: 'gmail',
    email: 'dev@gmail.com',
    name: null,
    profile: null,
    state: 'ready',
    isIdentity: false,
    error: null,
    lastCheckedAt: null,
    lastFound: 0,
    lastRunId: null,
    backfill: { phase: 'off', backTo: null, read: 0 },
    reads: { mail: true, files: false, calendar: true },
    readsApart: NONE,
    calendar: null,
    ...overrides,
  }
}

describe('readSwitches', () => {
  it("names the provider's drive and calendar and says which are on", () => {
    expect(readSwitches(account())).toEqual([
      { family: 'files', name: 'Google Drive', on: false, apart: false },
      { family: 'calendar', name: 'Google Calendar', on: true, apart: false },
    ])
    const outlook = readSwitches(
      account({
        provider: 'outlook',
        readsApart: { ...NONE, calendar: true },
      })
    )
    expect(outlook.map(s => s.name)).toEqual(['OneDrive', 'Outlook calendar'])
    expect(outlook[1]?.apart).toBe(true)
  })
})

describe('readChange', () => {
  it('turning one on signs in for it and everything already read', () => {
    expect(readChange(account(), 'files', true)).toEqual({
      kind: 'signIn',
      reads: { mail: true, files: true, calendar: true },
    })
  })

  it('turning one off only drops it', () => {
    expect(readChange(account(), 'calendar', false)).toEqual({
      kind: 'drop',
      reads: { mail: true, files: false, calendar: false },
    })
  })

  it('reconnects with everything, and adds mail to an account without it', () => {
    const noMail = account({ reads: { ...NONE, files: true } })
    expect(reconnectReads(noMail)).toEqual({ ...NONE, files: true })
    expect(addMailReads(noMail)).toEqual({ ...NONE, mail: true, files: true })
  })

  it('new connections tick files and calendar', () => {
    expect(DEFAULT_READS).toEqual({ mail: true, files: true, calendar: true })
  })
})

describe('calendarLine', () => {
  const check = {
    needsSignIn: false,
    error: null,
    lastCheckedAt: null,
    lastFound: 0,
    lastRunId: null,
  }

  it('says nothing without a calendar, then where its check is', () => {
    expect(calendarLine(null)).toBeNull()
    expect(calendarLine(check)).toEqual({
      key: 'email.reads.calendarNotChecked',
    })
    expect(calendarLine({ ...check, lastCheckedAt: 5, lastFound: 2 })).toEqual({
      key: 'workspace.lastChecked',
      when: 5,
      count: 2,
    })
    expect(calendarLine({ ...check, error: 'boom' })).toEqual({
      key: 'email.reads.calendarError',
      error: 'boom',
    })
    expect(calendarLine({ ...check, needsSignIn: true, error: 'x' })).toEqual({
      key: 'email.reads.calendarNeedsSignIn',
    })
  })

  it('shows the calendar check once an account reads its calendar', () => {
    expect(anyCalendar([account()])).toBe(true)
    expect(
      anyCalendar([account({ reads: { ...NONE, mail: true, files: true } })])
    ).toBe(false)
  })
})

describe('calendarIntervalOptions', () => {
  it('offers the usual intervals and keeps an unusual current one', () => {
    expect(calendarIntervalOptions(60)).toEqual([15, 30, 60, 120, 240])
    expect(calendarIntervalOptions(1440)).toEqual([15, 30, 60, 120, 240, 1440])
  })
})
