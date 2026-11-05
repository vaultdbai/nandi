import { describe, expect, it } from 'vitest'
import type { EmailProviderStatus } from '@/lib/tauri-bindings'
import {
  formatWatchList,
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
