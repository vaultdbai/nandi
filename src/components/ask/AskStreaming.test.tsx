import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, waitFor } from '@/test/test-utils'
import type { AskContext, AskHistoryEntry } from '@/lib/bindings'
import { visibleAnswer } from '@/hooks/useAsk'
import { AskBox } from './AskBox'
import { AskHistory } from './AskHistory'

const { mockClient, handlers } = vi.hoisted(() => {
  const handlers = new Set<(event: unknown) => void>()
  return {
    handlers,
    mockClient: {
      askStreaming: vi.fn(),
      listAskHistory: vi.fn(),
      supports: { attachments: false },
      subscribe: vi.fn((handler: (event: unknown) => void) => {
        handlers.add(handler)
        return {
          ready: Promise.resolve(),
          unsubscribe: () => handlers.delete(handler),
        }
      }),
    },
  }
})

vi.mock('@/lib/client', () => ({
  client: mockClient,
  isDesktop: () => false,
}))

const HOME: AskContext = { currentView: 'home', focusedArtifact: null }

function emit(event: string, payload: unknown) {
  act(() => {
    for (const handler of handlers) handler({ event, payload })
  })
}

describe('streaming answers', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockClient.listAskHistory.mockResolvedValue({ status: 'ok', data: [] })
  })

  it('shows the answer as it streams, then the full answer', async () => {
    mockClient.askStreaming.mockResolvedValue({ status: 'ok', data: 'req-1' })
    const user = userEvent.setup()
    render(<AskBox context={HOME} placeholder="Ask" />)
    await user.type(screen.getByRole('textbox'), 'Status?{Enter}')
    await waitFor(() => expect(mockClient.askStreaming).toHaveBeenCalled())
    expect(screen.getByText('Thinking')).toBeInTheDocument()

    // Another box's request is ignored
    emit('ask://streaming_delta', { requestId: 'other', delta: 'Nope' })
    emit('ask://streaming_delta', { requestId: 'req-1', delta: 'Two runs ' })
    expect(await screen.findByText('Two runs')).toBeInTheDocument()
    emit('ask://streaming_delta', {
      requestId: 'req-1',
      delta: 'are going.\nFOLLOWUPS: What failed?',
    })
    expect(await screen.findByText('Two runs are going.')).toBeInTheDocument()
    expect(screen.queryByText(/FOLLOWUPS/)).not.toBeInTheDocument()
    expect(screen.queryByText('Nope')).not.toBeInTheDocument()

    emit('ask://completed', {
      requestId: 'req-1',
      response: {
        kind: 'question',
        data: {
          answer: 'Two runs are going.',
          sources: [],
          followups: ['What failed?'],
        },
      },
      error: null,
    })
    expect(
      await screen.findByRole('button', { name: 'What failed?' })
    ).toBeInTheDocument()
    expect(screen.queryByText('Thinking')).not.toBeInTheDocument()
  })

  it('uses events that arrive before the request ID', async () => {
    mockClient.askStreaming.mockImplementation(async () => {
      emit('ask://completed', {
        requestId: 'req-early',
        response: { kind: 'clarification', data: 'Which one?' },
        error: null,
      })
      return { status: 'ok', data: 'req-early' }
    })
    const user = userEvent.setup()
    render(<AskBox context={HOME} placeholder="Ask" />)
    await user.type(screen.getByRole('textbox'), 'That{Enter}')
    expect(await screen.findByText('Which one?')).toBeInTheDocument()
  })

  it('shows a failed ask', async () => {
    mockClient.askStreaming.mockResolvedValue({ status: 'ok', data: 'req-2' })
    const user = userEvent.setup()
    render(<AskBox context={HOME} placeholder="Ask" />)
    await user.type(screen.getByRole('textbox'), 'Hm{Enter}')
    await waitFor(() => expect(mockClient.askStreaming).toHaveBeenCalled())
    emit('ask://completed', {
      requestId: 'req-2',
      response: null,
      error: 'No LLM API key is set',
    })
    expect(await screen.findByText('No LLM API key is set')).toBeInTheDocument()
  })

  it('keeps the follow-ups line out of the streamed text', () => {
    expect(visibleAnswer('Done.\nFOLLOWUPS: a | b')).toBe('Done.')
    expect(visibleAnswer('Partial ')).toBe('Partial')
  })
})

describe('AskHistory', () => {
  const entries: AskHistoryEntry[] = [
    {
      id: 'h1',
      requestText: 'What is running?',
      context: null,
      response: {
        kind: 'question',
        data: { answer: 'Three runs.', sources: [], followups: [] },
      },
      at: Date.now() - 60_000,
    },
    {
      id: 'h2',
      requestText: 'Review my code',
      context: null,
      response: {
        kind: 'action',
        data: {
          steps: [
            {
              kind: 'capability',
              target: 'code-review.python',
              description: 'Delegate to Python review',
            },
          ],
          estimatedDurationSeconds: 300,
          estimatedCostUsd: null,
          warnings: [],
          rejection: null,
        },
      },
      at: Date.now() - 120_000,
    },
  ]

  it('lists recent asks and opens what came back', async () => {
    mockClient.listAskHistory.mockResolvedValue({ status: 'ok', data: entries })
    const user = userEvent.setup()
    render(<AskHistory />)

    await user.click(
      await screen.findByRole('button', { name: /Recent asks \(2\)/ })
    )
    const list = screen.getByRole('list', { name: 'Recent asks' })
    expect(list).toHaveTextContent('What is running?')
    expect(list).toHaveTextContent('Answer')
    expect(list).toHaveTextContent('Plan')
    expect(mockClient.listAskHistory).toHaveBeenCalledWith(10)

    await user.click(screen.getByRole('button', { name: /What is running\?/ }))
    expect(screen.getByText('Three runs.')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /Review my code/ }))
    expect(screen.getByText('Delegate to Python review')).toBeInTheDocument()
  })
})
