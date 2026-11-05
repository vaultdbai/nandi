import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { EmailProviderStatus, EmailStatus } from '@/lib/bindings'
import { StepEmail } from './StepEmail'

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
    state: 'notConnected',
    error: null,
    lastCheckedAt: null,
    lastFound: 0,
    lastRunId: null,
    ...overrides,
  }
}

const status = (providers: EmailProviderStatus[]): EmailStatus => ({
  providers,
  intervalMinutes: 30,
  watch: [],
  nextCheckAt: null,
  checking: false,
})

const NAMES = ['getEmailStatus', 'setEmailSettings', 'startEmailSignIn']

describe('StepEmail', () => {
  const original: Commands = {}
  beforeEach(() => {
    for (const name of NAMES) original[name] = mocked[name]
  })
  afterEach(() => {
    for (const name of NAMES) mocked[name] = original[name]
  })

  it('lets the user pick both mailboxes and connect each', async () => {
    mocked.getEmailStatus = vi.fn(async () =>
      ok(status([provider(), provider({ provider: 'gmail' })]))
    )
    const user = userEvent.setup()
    render(<StepEmail onNext={vi.fn()} onBack={vi.fn()} />)
    await user.click(await screen.findByLabelText('Outlook'))
    await user.click(screen.getByLabelText('Gmail'))
    expect(
      screen.getByRole('button', { name: 'Connect Outlook' })
    ).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Connect Gmail' })
    ).toBeInTheDocument()
    expect(
      screen.getByText('Never sends, moves, deletes or marks anything read.')
    ).toBeInTheDocument()
  })

  it('defaults to 30 minutes and saves the choice when skipping', async () => {
    mocked.getEmailStatus = vi.fn(async () =>
      ok(status([provider(), provider({ provider: 'gmail' })]))
    )
    const save = vi.fn(async (settings: unknown) => {
      void settings
      return ok(status([provider()]))
    })
    mocked.setEmailSettings = save
    const onNext = vi.fn()
    const user = userEvent.setup()
    render(<StepEmail onNext={onNext} onBack={vi.fn()} />)
    expect(await screen.findByLabelText('30 minutes')).toBeChecked()
    expect(screen.getByLabelText('2 hours')).toBeInTheDocument()
    await user.click(screen.getByLabelText('15 minutes'))
    await user.click(screen.getByRole('button', { name: 'Skip for now' }))
    await waitFor(() => expect(onNext).toHaveBeenCalled())
    expect(save).toHaveBeenCalledWith({ intervalMinutes: 15, watch: [] })
  })

  it('shows a provider this build lacks calmly, without blocking', async () => {
    mocked.getEmailStatus = vi.fn(async () =>
      ok(
        status([
          provider({
            connected: true,
            state: 'ready',
            accountEmail: 'd@x.com',
          }),
          provider({ provider: 'gmail', available: false }),
        ])
      )
    )
    render(<StepEmail onNext={vi.fn()} onBack={vi.fn()} />)
    const gmail = (await screen.findByText('Gmail')).closest(
      '[data-provider]'
    ) as HTMLElement
    expect(
      within(gmail).getByText('Not available in this build')
    ).toBeInTheDocument()
    expect(screen.getByLabelText('Gmail')).toBeDisabled()
    expect(screen.getByText('Connected as d@x.com')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Continue' })).toBeEnabled()
  })
})
