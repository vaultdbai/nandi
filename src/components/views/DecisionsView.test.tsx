import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import i18n from '@/i18n/config'
import { commands } from '@/lib/tauri-bindings'
import type { DecisionRecord } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { DecisionsView } from './DecisionsView'

const ok = <T,>(data: T) => ({ status: 'ok' as const, data })
const t = (key: string) => i18n.t(key)

function record(overrides: Partial<DecisionRecord> = {}): DecisionRecord {
  return {
    at: 1_794_148_325_000,
    kind: 'gatewayHold',
    itemId: 'task-1',
    itemLabel: 'Sam Lee asks: the Q3 numbers',
    decision: 'refused',
    decidedBy: 'Dev Chechi',
    reason: 'Not before the board meeting',
    chatId: null,
    chatExists: false,
    ...overrides,
  }
}

describe('DecisionsView', () => {
  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
    commands.listDecisionMonths = vi.fn(async () => ok(['2026-11', '2026-10']))
    commands.listDecisions = vi.fn(async (month: string) =>
      ok(
        month === '2026-11'
          ? [
              record({
                chatId: 'inbox:gatewayHold:task-1',
                chatExists: true,
              }),
              record({
                at: 1_794_148_000_000,
                kind: 'approval',
                itemId: 'a1',
                itemLabel: 'run_report',
                decision: 'approved',
                reason: null,
                chatId: 'inbox:approval:gone',
                chatExists: false,
              }),
            ]
          : []
      )
    )
  })

  it('shows the newest month first with who, why and the badge', async () => {
    render(<DecisionsView />)

    expect(
      await screen.findByText('Sam Lee asks: the Q3 numbers')
    ).toBeInTheDocument()
    expect(commands.listDecisions).toHaveBeenCalledWith('2026-11', null)
    expect(screen.getByText('Not before the board meeting')).toBeVisible()
    expect(screen.getAllByText('by Dev Chechi')).toHaveLength(2)
    const rows = screen.getAllByRole('listitem')
    expect(within(rows[0] as HTMLElement).getByText('Refused')).toBeVisible()
    expect(within(rows[1] as HTMLElement).getByText('Approved')).toBeVisible()
    const months = screen.getByRole('combobox', { name: t('decisions.month') })
    expect(months).toHaveValue('2026-11')
  })

  it('sends the filters and search to the backend', async () => {
    const user = userEvent.setup()
    render(<DecisionsView />)
    await screen.findByText('run_report')

    await user.selectOptions(
      screen.getByRole('combobox', { name: t('decisions.kind.label') }),
      'approval'
    )
    await waitFor(() =>
      expect(commands.listDecisions).toHaveBeenLastCalledWith('2026-11', {
        kind: 'approval',
        decision: null,
        search: null,
      })
    )
    await user.selectOptions(
      screen.getByRole('combobox', { name: t('decisions.decision.label') }),
      'refused'
    )
    await user.type(
      screen.getByRole('searchbox', { name: t('decisions.search') }),
      'board'
    )
    await waitFor(() =>
      expect(commands.listDecisions).toHaveBeenLastCalledWith('2026-11', {
        kind: 'approval',
        decision: 'refused',
        search: 'board',
      })
    )
  })

  it('picks another month', async () => {
    const user = userEvent.setup()
    render(<DecisionsView />)
    await screen.findByText('run_report')

    await user.selectOptions(
      screen.getByRole('combobox', { name: t('decisions.month') }),
      '2026-10'
    )
    expect(await screen.findByTestId('decisions-no-match')).toHaveTextContent(
      t('decisions.noneThisMonth.title')
    )
  })

  it('says nothing matched when filters hide everything, and clears them', async () => {
    const user = userEvent.setup()
    commands.listDecisions = vi.fn(async (_month: string, filter) =>
      ok(filter ? [] : [record()])
    )
    render(<DecisionsView />)
    await screen.findByText('Sam Lee asks: the Q3 numbers')

    await user.type(
      screen.getByRole('searchbox', { name: t('decisions.search') }),
      'zzz'
    )
    const none = await screen.findByTestId('decisions-no-match')
    expect(none).toHaveTextContent(t('decisions.noMatch.title'))
    await user.click(
      within(none).getByRole('button', { name: t('decisions.clearFilters') })
    )
    expect(
      await screen.findByText('Sam Lee asks: the Q3 numbers')
    ).toBeInTheDocument()
  })

  it('opens the chat a decision came from while it exists, else shows its id', async () => {
    const user = userEvent.setup()
    render(<DecisionsView />)
    await screen.findByText('run_report')

    expect(screen.getByText('inbox:approval:gone')).toBeInTheDocument()
    expect(screen.getAllByText(t('decisions.fromChat'))).toHaveLength(2)
    await user.click(
      screen.getByRole('button', { name: t('decisions.fromChat') })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'inbox-thread',
      conversationId: 'inbox:gatewayHold:task-1',
    })
  })

  it('explains an empty history', async () => {
    commands.listDecisionMonths = vi.fn(async () => ok([]))
    render(<DecisionsView />)

    const empty = await screen.findByTestId('decisions-empty')
    expect(empty).toHaveTextContent(t('decisions.empty.title'))
    expect(empty).toHaveTextContent(t('decisions.empty.description'))
    expect(commands.listDecisions).not.toHaveBeenCalled()
    expect(
      screen.queryByRole('searchbox', { name: t('decisions.search') })
    ).not.toBeInTheDocument()
  })
})
