import { describe, expect, it } from 'vitest'
import type { EmailProviderStatus } from '@/lib/tauri-bindings'
import {
  backfillLine,
  estimateTotals,
  DEFAULT_INTERVAL_MINUTES,
  emailTodo,
  formatWatchList,
  intervalLabel,
  intervalOptions,
  mailboxCard,
  parseWatchList,
  problemKey,
} from './email-model'

function status(
  overrides: Partial<EmailProviderStatus> = {}
): EmailProviderStatus {
  return {
    provider: 'outlook',
    available: true,
    connected: false,
    accountName: null,
    accountEmail: null,
    state: 'notConnected',
    error: null,
    lastCheckedAt: null,
    lastFound: 0,
    lastRunId: null,
    backfill: { phase: 'off', backTo: null, read: 0 },
    ...overrides,
  }
}

describe('mailboxCard', () => {
  it('offers Connect only when the build can sign in', () => {
    expect(mailboxCard(status()).canConnect).toBe(true)
    const unavailable = mailboxCard(status({ available: false }))
    expect(unavailable.canConnect).toBe(false)
    expect(unavailable.statusKey).toBe('email.status.unavailable')
  })

  it('shows who is connected and what can be done', () => {
    const card = mailboxCard(
      status({
        connected: true,
        state: 'ready',
        accountName: 'Dev Chechi',
        accountEmail: 'dev@company.com',
      })
    )
    expect(card.account).toBe('Dev Chechi (dev@company.com)')
    expect(card).toMatchObject({
      canCheck: true,
      canPause: true,
      canResume: false,
      canDisconnect: true,
      canConnect: false,
      tone: 'ok',
    })
  })

  it('asks to reconnect when the sign-in ended', () => {
    const card = mailboxCard(
      status({
        provider: 'gmail',
        connected: true,
        state: 'needsSignIn',
        accountEmail: 'dev@gmail.com',
        error: 'invalid_grant',
      })
    )
    expect(card).toMatchObject({
      name: 'Gmail',
      canConnect: true,
      reconnect: true,
      tone: 'warning',
      error: 'invalid_grant',
      canCheck: false,
    })
  })

  it('offers Resume when paused', () => {
    const card = mailboxCard(status({ connected: true, state: 'paused' }))
    expect(card.canResume && !card.canPause && !card.canCheck).toBe(true)
  })
})

describe('watch list', () => {
  it('reads one entry per line or comma', () => {
    expect(parseWatchList(' a@x.com,\n\nvendor.com ; b.org ')).toEqual([
      'a@x.com',
      'vendor.com',
      'b.org',
    ])
    expect(formatWatchList(['a@x.com', 'vendor.com'])).toBe(
      'a@x.com\nvendor.com'
    )
  })
})

it('names a problem key per kind', () => {
  expect(problemKey('adminApproval')).toBe('email.problem.adminApproval')
})

describe('check interval', () => {
  it('defaults to 30 minutes and offers 5 minutes to 2 hours', () => {
    expect(DEFAULT_INTERVAL_MINUTES).toBe(30)
    expect(intervalOptions(30)).toEqual([5, 15, 30, 60, 120])
    expect(intervalOptions(10)).toEqual([5, 10, 15, 30, 60, 120])
  })

  it('says hours for whole hours', () => {
    const t = (key: string, { count }: { count: number }) => `${key}:${count}`
    expect(intervalLabel(t, 120)).toBe('email.hours:2')
    expect(intervalLabel(t, 30)).toBe('email.minutes:30')
    expect(intervalLabel(t, 90)).toBe('email.minutes:90')
  })
})

describe('emailTodo', () => {
  it('asks to connect while nothing is connected', () => {
    expect(emailTodo([status(), status({ provider: 'gmail' })])).toEqual({
      kind: 'connect',
      providers: ['outlook', 'gmail'],
    })
  })

  it('offers the other one quietly once one is connected', () => {
    expect(
      emailTodo([
        status({ connected: true, state: 'ready' }),
        status({ provider: 'gmail' }),
      ])
    ).toEqual({ kind: 'another', providers: ['gmail'] })
  })

  it('shows nothing when both are connected or none can be', () => {
    expect(
      emailTodo([
        status({ connected: true, state: 'ready' }),
        status({ provider: 'gmail', connected: true, state: 'ready' }),
      ])
    ).toEqual({ kind: 'none' })
    expect(
      emailTodo([
        status({ available: false }),
        status({ provider: 'gmail', available: false }),
      ])
    ).toEqual({ kind: 'none' })
  })
})

describe('backfillLine', () => {
  it('says where the read of older mail is, with its controls', () => {
    expect(backfillLine({ phase: 'off', backTo: null, read: 0 })).toEqual({
      key: null,
    })
    expect(
      backfillLine({ phase: 'reading', backTo: 1_700_000_000_000, read: 120 })
    ).toMatchObject({
      key: 'email.backfill.reading',
      when: 1_700_000_000_000,
      count: 120,
      canPause: true,
      canStop: true,
      canResume: false,
    })
    expect(backfillLine({ phase: 'reading', backTo: null, read: 0 }).key).toBe(
      'email.backfill.starting'
    )
    expect(backfillLine({ phase: 'paused', backTo: 1, read: 3 })).toMatchObject(
      { canResume: true, canPause: false }
    )
    expect(backfillLine({ phase: 'done', backTo: 1, read: 9 })).toMatchObject({
      key: 'email.backfill.done',
      canStop: false,
    })
  })

  it('adds up the estimates', () => {
    expect(
      estimateTotals([
        { provider: 'outlook', emails: 260, runs: 5 },
        { provider: 'gmail', emails: 60, runs: 1 },
      ])
    ).toEqual({ emails: 320, runs: 6 })
  })
})
