import { describe, expect, it, beforeEach, vi } from 'vitest'
import { commands } from '@/lib/tauri-bindings'
import { useAgentStore } from '@/store/agent-store'
import { queueEntry, run } from '@/test/fixtures'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { TurnQueueStrip } from './TurnQueue'

const setQueue = useAgentStore.getState().setQueue

describe('TurnQueueStrip', () => {
  beforeEach(() => {
    useAgentStore.setState(useAgentStore.getInitialState())
  })

  it('is hidden while the agent is idle with nothing queued', () => {
    const { container } = render(
      <TurnQueueStrip agentId="writer" agentName="Writer" />
    )
    expect(container).toBeEmptyDOMElement()
  })

  it('shows the current turn, truncated, without a queue badge', () => {
    setQueue('writer', [], run('r1', 'writer', { input: 'x'.repeat(80) }))
    render(<TurnQueueStrip agentId="writer" agentName="Writer" />)

    expect(
      screen.getByText(`Working on: ${'x'.repeat(60)}…`)
    ).toBeInTheDocument()
    expect(screen.queryByText(/queued/)).not.toBeInTheDocument()
  })

  it('opens the queue panel from the badge, with sources and actions', async () => {
    const user = userEvent.setup()
    setQueue(
      'writer',
      [
        queueEntry('q1', 'writer', { message: 'Follow up on the draft' }),
        queueEntry('q2', 'writer', {
          sourceType: 'workflow',
          sourceLabel: 'Daily Report · step 2',
          priority: 50,
          message: 'Summarize the news',
          position: 1,
        }),
      ],
      run('r1', 'writer', { input: 'Write the report' })
    )
    render(<TurnQueueStrip agentId="writer" agentName="Writer" />)

    await user.click(screen.getByRole('button', { name: 'Show queue' }))
    expect(screen.getByText('+2 queued')).toBeInTheDocument()
    expect(
      await screen.findByText('Follow up on the draft')
    ).toBeInTheDocument()
    // The current turn and the first queued message are both the user's
    expect(screen.getAllByText('You')).toHaveLength(2)
    expect(screen.getByText('Daily Report · step 2')).toBeInTheDocument()
    expect(screen.getByText('Priority 50')).toBeInTheDocument()

    const remove = screen.getAllByRole('button', { name: 'Remove from queue' })
    await user.click(remove[1] as HTMLElement)
    expect(commands.cancelQueuedRun).toHaveBeenCalledWith('q2')

    // Keyboard reorder: Alt+↑ moves the workflow step above the user's
    const handles = screen.getAllByRole('button', {
      name: 'Drag to reorder (Alt+↑/↓)',
    })
    ;(handles[1] as HTMLElement).focus()
    await user.keyboard('{Alt>}{ArrowUp}{/Alt}')
    expect(commands.reprioritizeQueuedRun).toHaveBeenCalledWith(
      'q2',
      expect.any(Number)
    )
    const priority = vi.mocked(commands.reprioritizeQueuedRun).mock
      .calls[0]?.[1]
    expect(priority).toBeLessThanOrEqual(10)
  })
})
