import { describe, expect, it } from 'vitest'
import type { EmailProviderStatus, MailboxStatus } from '@/lib/tauri-bindings'
import {
  accountLabel,
  addableProviders,
  backfillLine,
  canCheckNow,
  estimateTotals,
  DEFAULT_INTERVAL_MINUTES,
  emailTodo,
  formatWatchList,
  intervalLabel,
  intervalOptions,
  mailboxCard,
  mailboxOf,
  parseWatchList,
  problemKey,
  providerStatusKey,
} from './email-model'

function status(
  overrides: Partial<EmailProviderStatus> = {}
): EmailProviderStatus {
  return {
    provider: 'outlook',
    available: true,
    connected: false,
    ...overrides,
  }
}

function mailbox(overrides: Partial<MailboxStatus> = {}): MailboxStatus {
  return {
    provider: 'outlook',
    email: 'dev@company.com',
    name: null,
    profile: null,
    state: 'ready',
    isIdentity: false,
    error: null,
    lastCheckedAt: null,
    lastFound: 0,
    lastRunId: null,
    backfill: { phase: 'off', backTo: null, read: 0 },
    reads: { mail: true, files: false, calendar: false },
    readsApart: { mail: false, files: false, calendar: false },
    calendar: null,
    ...overrides,
  }
}

describe('mailboxOf', () => {
  it('names a mailbox by provider and address', () => {
    expect(
      mailboxOf(mailbox({ provider: 'gmail', email: 'dev@gmail.com' }))
    ).toEqual({ provider: 'gmail', email: 'dev@gmail.com' })
  })
})

describe('accountLabel', () => {
  it('shows the name with the address, or the address alone', () => {
    expect(accountLabel({ name: 'Dev Chechi', email: 'dev@company.com' })).toBe(
      'Dev Chechi (dev@company.com)'
    )
    expect(accountLabel({ name: null, email: 'dev@company.com' })).toBe(
      'dev@company.com'
    )
  })
})

describe('providers a mailbox can be added from', () => {
  it('lists the providers this build can sign in to', () => {
    expect(addableProviders([status(), status({ provider: 'gmail' })])).toEqual(
      ['outlook', 'gmail']
    )
    expect(
      addableProviders([
        status({ available: false }),
        status({ provider: 'gmail' }),
      ])
    ).toEqual(['gmail'])
  })

  it('says why a provider with no mailbox has none', () => {
    expect(providerStatusKey(status())).toBe('email.status.notConnected')
    expect(providerStatusKey(status({ available: false }))).toBe(
      'email.status.unavailable'
    )
  })
})

describe('mailboxCard', () => {
  it('an account without mail reads only its files and calendar', () => {
    const card = mailboxCard(
      mailbox({ reads: { mail: false, files: true, calendar: false } })
    )
    expect(card).toMatchObject({
      statusKey: 'email.status.noMail',
      canPause: false,
      reconnect: false,
    })
    expect(
      canCheckNow([
        mailbox({ reads: { mail: false, files: true, calendar: false } }),
      ])
    ).toBe(false)
  })

  it('a calendar whose sign-in ended offers Reconnect', () => {
    const card = mailboxCard(
      mailbox({
        reads: { mail: true, files: false, calendar: true },
        calendar: {
          needsSignIn: true,
          error: 'expired',
          lastCheckedAt: null,
          lastFound: 0,
          lastRunId: null,
        },
      })
    )
    expect(card.reconnect).toBe(true)
  })

  it('shows who is connected and what can be done', () => {
    const card = mailboxCard(
      mailbox({ name: 'Dev Chechi', email: 'dev@company.com' })
    )
    expect(card.account).toBe('Dev Chechi (dev@company.com)')
    expect(card).toMatchObject({
      canPause: true,
      canResume: false,
      reconnect: false,
      tone: 'ok',
    })
  })

  it('asks to reconnect when the sign-in ended', () => {
    const card = mailboxCard(
      mailbox({
        provider: 'gmail',
        state: 'needsSignIn',
        email: 'dev@gmail.com',
        error: 'invalid_grant',
      })
    )
    expect(card).toMatchObject({
      name: 'Gmail',
      reconnect: true,
      tone: 'warning',
      error: 'invalid_grant',
      canPause: false,
    })
  })

  it('offers Resume when paused', () => {
    const card = mailboxCard(mailbox({ state: 'paused' }))
    expect(card.canResume && !card.canPause).toBe(true)
  })
})

describe('canCheckNow', () => {
  it('needs a mailbox that is being read', () => {
    expect(canCheckNow([])).toBe(false)
    expect(
      canCheckNow([
        mailbox({ state: 'paused' }),
        mailbox({ state: 'needsSignIn' }),
      ])
    ).toBe(false)
    expect(
      canCheckNow([mailbox({ state: 'paused' }), mailbox({ state: 'error' })])
    ).toBe(true)
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
      emailTodo([status({ connected: true }), status({ provider: 'gmail' })])
    ).toEqual({ kind: 'another', providers: ['gmail'] })
  })

  it('shows nothing when both are connected or none can be', () => {
    expect(
      emailTodo([
        status({ connected: true }),
        status({ provider: 'gmail', connected: true }),
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
