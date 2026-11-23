import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type {
  EmailProviderStatus,
  EmailStatus,
  MailboxStatus,
} from '@/lib/bindings'
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
    ...overrides,
  }
}

function status(
  providers: EmailProviderStatus[],
  mailboxes: MailboxStatus[] = []
): EmailStatus {
  return {
    providers,
    mailboxes,
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

function row(email: string): HTMLElement {
  const found = document.querySelector(`[data-mailbox="${email}"]`)
  if (!found) throw new Error(`No row for ${email}`)
  return found as HTMLElement
}

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
    expect(screen.getByText('No mailbox is connected yet.')).toBeInTheDocument()
    const gmail = document.querySelector('[data-provider="gmail"]')
    expect(gmail).not.toBeNull()
    expect(
      within(gmail as HTMLElement).getByText(/Not available in this build/)
    ).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /Gmail/ })).toBeNull()
  })

  it('shows the account and checks now, pauses and signs out', async () => {
    const connected = status(
      [provider({ connected: true }), provider({ provider: 'gmail' })],
      [
        mailbox({
          name: 'Dev Chechi',
          lastCheckedAt: Date.now() - 60_000,
          lastFound: 2,
        }),
      ]
    )
    mocked.getEmailStatus = vi.fn(async () => ok(connected))
    const check = vi.fn(async () => ok({ ...connected, checking: true }))
    const pause = vi.fn(async () => ok(connected))
    const disconnect = vi.fn(async () =>
      ok(status([provider(), provider({ provider: 'gmail' })]))
    )
    mocked.checkEmailNow = check
    mocked.setEmailPaused = pause
    mocked.disconnectEmail = disconnect
    const user = userEvent.setup()
    render(<EmailSection />)

    expect(
      await screen.findByText('Dev Chechi (dev@company.com)')
    ).toBeInTheDocument()
    expect(screen.getByText(/2 new emails for your notes/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Check now' }))
    await waitFor(() => expect(check).toHaveBeenCalled())
    await user.click(screen.getByRole('button', { name: 'Pause' }))
    await waitFor(() =>
      expect(pause).toHaveBeenCalledWith(
        { provider: 'outlook', email: 'dev@company.com' },
        true
      )
    )
    await user.click(screen.getByRole('button', { name: 'Sign out' }))
    const dialog = await screen.findByRole('alertdialog')
    await user.click(within(dialog).getByRole('button', { name: 'Sign out' }))
    await waitFor(() =>
      expect(disconnect).toHaveBeenCalledWith({
        provider: 'outlook',
        email: 'dev@company.com',
      })
    )
  })

  it('lists every mailbox with its own controls and marks the identity', async () => {
    const both = status(
      [
        provider({ connected: true }),
        provider({ provider: 'gmail', connected: true }),
      ],
      [
        mailbox({ email: 'dev@company.com', isIdentity: true }),
        mailbox({
          provider: 'gmail',
          email: 'home@gmail.com',
          state: 'paused',
        }),
        mailbox({ provider: 'gmail', email: 'side@gmail.com' }),
      ]
    )
    mocked.getEmailStatus = vi.fn(async () => ok(both))
    const pause = vi.fn(async () => ok(both))
    const disconnect = vi.fn(async () => ok(both))
    mocked.setEmailPaused = pause
    mocked.disconnectEmail = disconnect
    const user = userEvent.setup()
    render(<EmailSection />)

    await screen.findByText('home@gmail.com')
    expect(document.querySelectorAll('[data-mailbox]')).toHaveLength(3)
    // Only the identity mailbox is marked
    expect(within(row('dev@company.com')).getByText('Identity')).toBeVisible()
    expect(within(row('home@gmail.com')).queryByText('Identity')).toBeNull()
    expect(within(row('side@gmail.com')).queryByText('Identity')).toBeNull()
    // Each row acts on its own mailbox
    await user.click(
      within(row('home@gmail.com')).getByRole('button', { name: 'Resume' })
    )
    await waitFor(() =>
      expect(pause).toHaveBeenCalledWith(
        { provider: 'gmail', email: 'home@gmail.com' },
        false
      )
    )
    await user.click(
      within(row('side@gmail.com')).getByRole('button', { name: 'Sign out' })
    )
    const dialog = await screen.findByRole('alertdialog')
    expect(within(dialog).queryByText(/identity/)).toBeNull()
    await user.click(within(dialog).getByRole('button', { name: 'Sign out' }))
    await waitFor(() =>
      expect(disconnect).toHaveBeenCalledWith({
        provider: 'gmail',
        email: 'side@gmail.com',
      })
    )
    // Add another account, for each provider
    expect(
      screen.getByRole('button', { name: 'Add another Outlook account' })
    ).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Add another Gmail account' })
    ).toBeInTheDocument()
  })

  it('says what signing out of the identity mailbox does', async () => {
    mocked.getEmailStatus = vi.fn(async () =>
      ok(
        status([provider({ connected: true })], [mailbox({ isIdentity: true })])
      )
    )
    const user = userEvent.setup()
    render(<EmailSection />)
    await user.click(await screen.findByRole('button', { name: 'Sign out' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(
      within(dialog).getByText(/identity\. Signing out does not change it/)
    ).toBeInTheDocument()
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
        status(
          [provider({ provider: 'gmail', connected: true })],
          [
            mailbox({
              provider: 'gmail',
              email: 'dev@gmail.com',
              state: 'needsSignIn',
            }),
          ]
        )
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
