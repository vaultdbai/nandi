import { describe, expect, it } from 'vitest'
import type { WorkspaceAccountStatus } from '@/lib/bindings'
import { calendarIntervalOptions, workspaceCard } from './workspace-model'

function account(
  overrides: Partial<WorkspaceAccountStatus> = {}
): WorkspaceAccountStatus {
  return {
    kind: 'outlookCalendar',
    id: 'outlook-calendar',
    family: 'calendar',
    available: true,
    connected: false,
    accountEmail: null,
    needsSignIn: false,
    error: null,
    lastCheckedAt: null,
    lastFound: 0,
    lastRunId: null,
    ...overrides,
  }
}

describe('workspaceCard', () => {
  it('offers Connect when not connected', () => {
    const card = workspaceCard(account())
    expect(card.statusKey).toBe('notConnected')
    expect(card.canConnect).toBe(true)
    expect(card.canDisconnect).toBe(false)
    expect(card.name).toBe('Outlook calendar')
  })

  it('offers nothing when this build cannot connect it', () => {
    const card = workspaceCard(account({ available: false }))
    expect(card.statusKey).toBe('notAvailable')
    expect(card.canConnect).toBe(false)
  })

  it('checks a connected calendar, not a drive', () => {
    const calendar = workspaceCard(
      account({ connected: true, accountEmail: 'a@b.c' })
    )
    expect(calendar.statusKey).toBe('connected')
    expect(calendar.canCheck).toBe(true)
    expect(calendar.account).toBe('a@b.c')
    const drive = workspaceCard(
      account({ kind: 'oneDrive', family: 'files', connected: true })
    )
    expect(drive.canCheck).toBe(false)
    expect(drive.canDisconnect).toBe(true)
  })

  it('asks to sign in again, or shows the error', () => {
    expect(
      workspaceCard(account({ connected: true, needsSignIn: true }))
    ).toMatchObject({ statusKey: 'needsSignIn', reconnect: true })
    expect(
      workspaceCard(account({ connected: true, error: 'boom' }))
    ).toMatchObject({ statusKey: 'error', tone: 'error', error: 'boom' })
  })
})

describe('calendarIntervalOptions', () => {
  it('offers the usual intervals and keeps an unusual current one', () => {
    expect(calendarIntervalOptions(60)).toEqual([15, 30, 60, 120, 240])
    expect(calendarIntervalOptions(1440)).toEqual([15, 30, 60, 120, 240, 1440])
  })
})
