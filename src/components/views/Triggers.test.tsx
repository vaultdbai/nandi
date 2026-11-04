import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { Trigger, TriggerFire } from '@/lib/tauri-bindings'
import { agentInfo, T0 } from '@/test/fixtures'
import { useNavigationStore } from '@/store/navigation-store'
import { TriggerDetailView } from './TriggerDetailView'
import { TriggersListView } from './TriggersListView'

function trigger(overrides: Partial<Trigger> = {}): Trigger {
  return {
    id: 'daily-digest',
    name: 'Daily Digest',
    enabled: true,
    sourceType: 'cron',
    sourceConfig: { schedule: '0 9 * * 1-5' },
    targetType: 'agent',
    targetId: 'research-agent',
    inputTemplate: 'Summarize the news',
    priority: 80,
    rateLimitSeconds: null,
    createdAt: T0,
    updatedAt: T0,
    lastFiredAt: null,
    hasSecret: false,
    ...overrides,
  }
}

const HOOK = trigger({
  id: 'orders',
  name: 'New orders',
  sourceType: 'webhook',
  sourceConfig: {},
  hasSecret: true,
  priority: 60,
})

function fire(overrides: Partial<TriggerFire> = {}): TriggerFire {
  return {
    id: 'f1',
    triggerId: 'orders',
    firedAt: T0,
    payload: { order: { id: 42 } },
    runId: 'run-12345678',
    status: 'queued',
    error: null,
    ...overrides,
  }
}

const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

describe('TriggersListView', () => {
  beforeEach(() => {
    vi.mocked(commands.listTriggers).mockResolvedValue(ok([trigger(), HOOK]))
  })

  it('lists triggers in words, filters by source, and toggles or tests them', async () => {
    const user = userEvent.setup()
    render(<TriggersListView />)

    expect(await screen.findByText('Daily Digest')).toBeInTheDocument()
    expect(screen.getByText('New orders')).toBeInTheDocument()
    // The schedule in words; the raw cron is only in the tooltip
    expect(
      screen.getByText('At 09:00, Monday through Friday')
    ).toBeInTheDocument()
    expect(screen.queryByText('0 9 * * 1-5')).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: /Webhook/ }))
    expect(screen.queryByText('Daily Digest')).not.toBeInTheDocument()
    expect(screen.getByText('New orders')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'All' }))

    const row = screen.getByText('Daily Digest').closest('tr') as HTMLElement
    await user.click(within(row).getByRole('switch'))
    expect(commands.setTriggerEnabled).toHaveBeenCalledWith(
      'daily-digest',
      false
    )

    // Test… warns that it really runs, and fires only on confirm
    await user.click(
      within(row).getByRole('button', { name: 'More for Daily Digest' })
    )
    await user.click(await screen.findByRole('menuitem', { name: 'Test…' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(within(dialog).getByText(/This really fires/)).toBeInTheDocument()
    expect(commands.testFireTrigger).not.toHaveBeenCalled()
    await user.click(
      within(dialog).getByRole('button', { name: 'Fire it now' })
    )
    expect(commands.testFireTrigger).toHaveBeenCalledWith('daily-digest', {
      test: true,
    })
  })

  it('creates a webhook trigger and shows its secret once', async () => {
    const user = userEvent.setup()
    vi.mocked(commands.listAgents).mockResolvedValue(
      ok([agentInfo('research-agent', 'Research')])
    )
    vi.mocked(commands.getWebOrigin).mockResolvedValue(
      ok('http://127.0.0.1:7777')
    )
    vi.mocked(commands.createTrigger).mockResolvedValueOnce(
      ok({ trigger: HOOK, secret: 'a'.repeat(64) })
    )
    render(<TriggersListView />)
    await screen.findByText('Daily Digest')

    await user.click(screen.getByRole('button', { name: 'New Trigger' }))
    const dialog = await screen.findByRole('dialog')
    await user.click(within(dialog).getByRole('tab', { name: /Webhook/ }))
    await user.type(within(dialog).getByLabelText('Name'), 'New orders')
    const create = within(dialog).getByRole('button', {
      name: 'Create trigger',
    })
    expect(create).toBeDisabled()
    await user.selectOptions(
      within(dialog).getByRole('combobox', { name: 'Run' }),
      'research-agent'
    )
    await user.click(create)

    expect(commands.createTrigger).toHaveBeenCalledWith(
      expect.objectContaining({
        name: 'New orders',
        sourceType: 'webhook',
        sourceConfig: {},
        targetType: 'agent',
        targetId: 'research-agent',
        priority: null,
      })
    )
    expect(await screen.findByText('Copy the secret now')).toBeInTheDocument()
    expect(within(dialog).getByText('a'.repeat(64))).toBeInTheDocument()
    expect(
      await within(dialog).findByText(
        'http://127.0.0.1:7777/api/triggers/webhook/orders'
      )
    ).toBeInTheDocument()
    // A ready-made command to try it
    expect(within(dialog).getByText(/^curl /)).toBeInTheDocument()
  })
})

describe('TriggerDetailView', () => {
  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
    vi.mocked(commands.getTrigger).mockResolvedValue(ok(HOOK))
    vi.mocked(commands.listTriggerFires).mockResolvedValue(
      ok([
        fire(),
        fire({
          id: 'f0',
          status: 'rateLimited',
          runId: null,
          payload: { order: { id: 41 } },
          error: 'Rate limited until 2026-10-06T10:00:00Z',
        }),
      ])
    )
  })

  it('asks before making a new secret, then shows it once', async () => {
    const user = userEvent.setup()
    vi.mocked(commands.rotateTriggerSecret).mockResolvedValueOnce(
      ok({ trigger: HOOK, secret: 'b'.repeat(64) })
    )
    render(<TriggerDetailView triggerId="orders" />)

    // No origin yet: nothing wrong to copy
    expect(
      await screen.findByText("Starting Nandi's local server…")
    ).toBeInTheDocument()
    expect(screen.getByText(/The secret is hidden/)).toBeInTheDocument()
    await user.click(
      screen.getByRole('button', { name: 'Generate a new secret' })
    )
    const confirm = await screen.findByRole('alertdialog')
    expect(
      within(confirm).getByText(/stops working at once/)
    ).toBeInTheDocument()
    expect(commands.rotateTriggerSecret).not.toHaveBeenCalled()
    await user.click(
      within(confirm).getByRole('button', { name: 'Make a new secret' })
    )
    expect(commands.rotateTriggerSecret).toHaveBeenCalledWith('orders')
    expect(await screen.findByText('b'.repeat(64))).toBeInTheDocument()
  })

  it('lists firings with their results and what each received', async () => {
    const user = userEvent.setup()
    render(<TriggerDetailView triggerId="orders" />)
    await screen.findByRole('heading', { name: 'New orders' })

    await user.click(screen.getByRole('tab', { name: 'Firings' }))
    expect(await screen.findByText('Queued')).toBeInTheDocument()
    expect(screen.getByText('Rate limited')).toBeInTheDocument()

    await user.click(screen.getByText('Rate limited'))
    expect(screen.getByText(/"id": 41/)).toBeInTheDocument()

    // An agent firing opens that run
    await user.click(screen.getByRole('button', { name: /^Open / }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'run-detail',
      runId: 'run-12345678',
    })
  })

  it("won't save a schedule the scheduler can't read", async () => {
    const user = userEvent.setup()
    vi.mocked(commands.getTrigger).mockResolvedValue(ok(trigger()))
    vi.mocked(commands.previewCron).mockImplementation(async schedule =>
      schedule === 'nope'
        ? ok({ valid: false, error: 'bad', next: [] })
        : ok({ valid: true, error: null, next: [T0, T0 + 86_400_000] })
    )
    render(<TriggerDetailView triggerId="daily-digest" />)

    expect(await screen.findByTestId('cron-next')).toBeInTheDocument()
    await user.click(screen.getByRole('radio', { name: 'Custom' }))
    const cron = screen.getByLabelText('Cron expression')
    await user.clear(cron)
    await user.type(cron, 'nope')

    expect(
      await screen.findByText(/Nandi can't read this schedule/)
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Save changes' })).toBeDisabled()
    // Unsaved changes hold navigation
    expect(useNavigationStore.getState().leaveGuarded).toBe(true)
  })
  it('shows who is on duty next, with their backup', async () => {
    vi.mocked(commands.getTrigger).mockResolvedValue(
      ok(
        trigger({
          id: 'rota',
          name: 'Support rota',
          sourceType: 'calendar',
          sourceConfig: {
            times: ['09:00'],
            people: ['alice', 'bob'],
            startDate: '2026-10-26',
          },
        })
      )
    )
    vi.mocked(commands.previewCalendar).mockResolvedValue(
      ok({
        valid: true,
        error: null,
        next: [
          { at: T0, person: 'alice', backup: 'bob', coveringFor: null },
          {
            at: T0 + 86_400_000,
            person: 'dana',
            backup: 'bob',
            coveringFor: 'alice',
          },
        ],
      })
    )
    render(<TriggerDetailView triggerId="rota" />)

    const next = await screen.findByTestId('calendar-next')
    expect(within(next).getByText('alice')).toBeInTheDocument()
    expect(within(next).getByText('dana (for alice)')).toBeInTheDocument()
    expect(within(next).getAllByText('backup: bob')).toHaveLength(2)
    expect(screen.getByLabelText('People who take turns')).toHaveValue(
      'alice, bob'
    )
  })
})
