import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { EmailProviderStatus, EmailStatus } from '@/lib/bindings'
import { EmailSection } from './EmailSection'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

function provider(
  overrides: Partial<EmailProviderStatus> = {}
): EmailProviderStatus {
  return {
    provider: 'outlook',
    available: true,
    connected: false,
    accountName: null,
    accountEmail: null,
    accountProfile: null,
    state: 'notConnected',
    error: null,
    lastCheckedAt: null,
    lastFound: 0,
    lastRunId: null,
    backfill: { phase: 'off', backTo: null, read: 0 },
    ...overrides,
  }
}

function status(providers: EmailProviderStatus[]): EmailStatus {
  return {
    providers,
    intervalMinutes: 5,
    watch: [],
    nextCheckAt: null,
    checking: false,
    backfillRange: 'everything',
  }
}

const NAMES = [
  'getEmailStatus',
  'startEmailSignIn',
  'pollEmailSignIn',
  'checkEmailNow',
  'setEmailPaused',
  'disconnectEmail',
  'setEmailSettings',
]

describe('EmailSection', () => {
  const original: Commands = {}

  beforeEach(() => {
    for (const name of NAMES) original[name] = mocked[name]
  })

  afterEach(() => {
    for (const name of NAMES) mocked[name] = original[name]
  })

  it('says what is read and offers Connect for each available provider', async () => {
    mocked.getEmailStatus = vi.fn(async () =>
      ok(
        status([provider(), provider({ provider: 'gmail', available: false })])
      )
    )
    render(<EmailSection />)
    expect(
      await screen.findByRole('button', { name: 'Connect Outlook' })
    ).toBeInTheDocument()
    expect(
      screen.getByText('Never sends, moves, deletes or marks anything read.')
    ).toBeInTheDocument()
    const gmail = document.querySelector('[data-provider="gmail"]')
    expect(gmail).not.toBeNull()
    expect(
      within(gmail as HTMLElement).getByText('Not available in this build')
    ).toBeInTheDocument()
    expect(
      within(gmail as HTMLElement).queryByRole('button', { name: /Connect/ })
    ).toBeNull()
  })

  it('shows the account and checks now, pauses and disconnects', async () => {
    const connected = status([
      provider({
        connected: true,
        state: 'ready',
        accountName: 'Dev Chechi',
        accountEmail: 'dev@company.com',
        lastCheckedAt: Date.now() - 60_000,
        lastFound: 2,
      }),
      provider({ provider: 'gmail' }),
    ])
    mocked.getEmailStatus = vi.fn(async () => ok(connected))
    const check = vi.fn(async () => ok({ ...connected, checking: true }))
    const pause = vi.fn(async () => ok(connected))
    const disconnect = vi.fn(async () => ok(status([provider()])))
    mocked.checkEmailNow = check
    mocked.setEmailPaused = pause
    mocked.disconnectEmail = disconnect
    const user = userEvent.setup()
    render(<EmailSection />)

    expect(
      await screen.findByText('Connected as Dev Chechi (dev@company.com)')
    ).toBeInTheDocument()
    expect(screen.getByText(/2 new emails for your notes/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Check now' }))
    await waitFor(() => expect(check).toHaveBeenCalled())
    await user.click(screen.getByRole('button', { name: 'Pause' }))
    await waitFor(() => expect(pause).toHaveBeenCalledWith('outlook', true))
    await user.click(
      screen.getByRole('button', { name: 'Stop reading my email' })
    )
    const dialog = await screen.findByRole('alertdialog')
    await user.click(
      within(dialog).getByRole('button', { name: 'Stop reading my email' })
    )
    await waitFor(() => expect(disconnect).toHaveBeenCalledWith('outlook'))
  })

  it('explains an admin approval and offers the link', async () => {
    mocked.getEmailStatus = vi.fn(async () => ok(status([provider()])))
    mocked.startEmailSignIn = vi.fn(async () =>
      ok({
        deviceCode: 'x',
        userCode: '',
        verificationUri: 'https://login.example/authorize',
        interval: 1,
        expiresAt: 0,
      })
    )
    mocked.pollEmailSignIn = vi.fn(async () =>
      ok({
        phase: 'failed',
        problem: {
          kind: 'adminApproval',
          detail: 'AADSTS65001',
          adminConsentUrl: 'https://login.microsoftonline.com/x',
        },
      })
    )
    const user = userEvent.setup()
    render(<EmailSection />)
    await user.click(
      await screen.findByRole('button', { name: 'Connect Outlook' })
    )
    expect(
      await screen.findByText(/needs an IT admin to approve Nandi/, undefined, {
        timeout: 4000,
      })
    ).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Copy link for IT' })
    ).toBeInTheDocument()
  })

  it('offers Reconnect when the sign-in ended', async () => {
    mocked.getEmailStatus = vi.fn(async () =>
      ok(
        status([
          provider({
            provider: 'gmail',
            connected: true,
            state: 'needsSignIn',
            accountEmail: 'dev@gmail.com',
          }),
        ])
      )
    )
    render(<EmailSection />)
    expect(
      await screen.findByRole('button', { name: 'Reconnect' })
    ).toBeInTheDocument()
    expect(
      screen.getByText('Sign in again to keep reading')
    ).toBeInTheDocument()
  })
})
