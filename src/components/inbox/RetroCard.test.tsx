import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { RetroItem } from '@/lib/bindings'
import { RetroCard } from './RetroCard'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })
const mocks = () =>
  commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const DIFF = [
  '--- a/.nandi/agents/writer/agent.md',
  '+++ b/.nandi/agents/writer/agent.md',
  '@@ -1 +1,2 @@',
  ' Draft posts.',
  '+Keep them under 80 words.',
].join('\n')

const item = (overrides: Partial<RetroItem> = {}): RetroItem => ({
  id: 'p1',
  agentId: 'writer',
  agentName: 'Writer',
  target: 'agent',
  path: '.nandi/agents/writer/agent.md',
  reason: 'You shortened 4 of 5 drafts',
  runs: ['r1', 'r2'],
  diff: DIFF,
  status: 'pending',
  createdAt: 1000,
  decidedAt: null,
  expiresAt: null,
  commitId: null,
  verdict: null,
  rates: null,
  needsNote: false,
  ...overrides,
})

beforeEach(() => {
  Object.assign(commands, {
    actOnRetroProposal: ok(null),
    listRetroItems: ok([]),
  })
})

describe('RetroCard', () => {
  it('shows why, the change and the runs, and applies on yes', async () => {
    const user = userEvent.setup()
    render(<RetroCard item={item()} />)
    expect(screen.getByText(/You shortened 4 of 5 drafts/)).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: /run \d/ })).toHaveLength(2)
    await user.click(screen.getByText('Show the change'))
    expect(screen.getByTestId('retro-diff')).toHaveTextContent(
      '+ Keep them under 80 words.'
    )
    expect(screen.queryByRole('button', { name: 'Undo' })).toBeNull()
    await user.click(screen.getByRole('button', { name: 'Apply' }))
    await waitFor(() =>
      expect(mocks().actOnRetroProposal).toHaveBeenCalledWith('p1', 'apply')
    )
  })

  it('offers the undo once applied, and the rates when it did not help', async () => {
    const user = userEvent.setup()
    const { rerender } = render(
      <RetroCard item={item({ status: 'applied', decidedAt: 2000 })} />
    )
    expect(screen.queryByRole('button', { name: 'Apply' })).toBeNull()
    await user.click(screen.getByRole('button', { name: 'Undo' }))
    await waitFor(() =>
      expect(mocks().actOnRetroProposal).toHaveBeenCalledWith('p1', 'undo')
    )
    rerender(
      <RetroCard
        item={item({
          status: 'applied',
          decidedAt: 2000,
          verdict: 'worse',
          needsNote: true,
          rates: {
            before: { decisions: 4, corrections: 2 },
            after: { decisions: 5, corrections: 3 },
          },
        })}
      />
    )
    expect(screen.getByText("This change to Writer didn't help")).toBeVisible()
    expect(screen.getByTestId('retro-rates')).toHaveTextContent('50%')
    expect(screen.getByTestId('retro-rates')).toHaveTextContent('60%')
    await user.click(screen.getByRole('button', { name: 'Keep it' }))
    await waitFor(() =>
      expect(mocks().actOnRetroProposal).toHaveBeenCalledWith('p1', 'keep')
    )
  })
})
