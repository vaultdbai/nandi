import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { render, screen } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { EmailProviderStatus, EmailStatus } from '@/lib/bindings'
import { EmailTodo } from './EmailTodo'

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
    backfill: { phase: 'off', backTo: null, read: 0 },
    ...overrides,
  }
}

const status = (providers: EmailProviderStatus[]): EmailStatus => ({
  providers,
  intervalMinutes: 30,
  watch: [],
  nextCheckAt: null,
  checking: false,
  backfillRange: 'everything',
})

describe('EmailTodo', () => {
  let original: unknown
  beforeEach(() => {
    original = mocked.getEmailStatus
  })
  afterEach(() => {
    mocked.getEmailStatus = original
  })

  it('asks to connect while no mailbox is connected', async () => {
    mocked.getEmailStatus = vi.fn(async () =>
      ok(status([provider(), provider({ provider: 'gmail' })]))
    )
    render(<EmailTodo />)
    expect(
      await screen.findByText(
        'Connect your email so Chief of Staff can keep your notes.'
      )
    ).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Connect Outlook' })
    ).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Connect Gmail' })
    ).toBeInTheDocument()
  })

  it('goes away once one is connected, offering the other quietly', async () => {
    mocked.getEmailStatus = vi.fn(async () =>
      ok(
        status([
          provider({ connected: true, state: 'ready' }),
          provider({ provider: 'gmail' }),
        ])
      )
    )
    render(<EmailTodo />)
    expect(
      await screen.findByText('You can also connect Gmail.')
    ).toBeInTheDocument()
    expect(screen.queryByTestId('home-email-todo')).toBeNull()
  })

  it('shows nothing when both are connected', async () => {
    const both = vi.fn(async () =>
      ok(
        status([
          provider({ connected: true, state: 'ready' }),
          provider({ provider: 'gmail', connected: true, state: 'ready' }),
        ])
      )
    )
    mocked.getEmailStatus = both
    const { container } = render(<EmailTodo />)
    await vi.waitFor(() => expect(both).toHaveBeenCalled())
    expect(container.querySelector('[data-testid^="home-email"]')).toBeNull()
  })
})
