import { beforeEach, describe, expect, it, vi } from 'vitest'
import { streamAsksThroughAsk } from '@/test/ask-stream'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import type { AskContext, AskResponse } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { AskBox } from './AskBox'

const { mockClient } = vi.hoisted(() => ({
  mockClient: {
    ask: vi.fn(),
    supports: { attachments: false },
    subscribe: vi.fn(() => ({
      ready: Promise.resolve(),
      unsubscribe: vi.fn(),
    })),
  },
}))

vi.mock('@/lib/client', () => ({
  client: mockClient,
  isDesktop: () => false,
}))

const HOME: AskContext = { currentView: 'home', focusedArtifact: null }

const answer: AskResponse = {
  kind: 'question',
  data: {
    answer: 'Run `r1` is still going for **writer**.',
    sources: [{ kind: 'agent', id: 'writer', label: 'Writer' }],
    followups: ['What failed today?'],
  },
}

async function ask(text: string) {
  const user = userEvent.setup()
  await user.type(screen.getByRole('textbox'), `${text}{Enter}`)
  return user
}

// The ask box streams; answers come from the `ask` mock
streamAsksThroughAsk(mockClient)

describe('AskBox', () => {
  beforeEach(() => {
    mockClient.ask.mockReset()
    useNavigationStore.setState(useNavigationStore.getInitialState())
  })

  it('sends the ask with its context and shows the answer inline', async () => {
    mockClient.ask.mockResolvedValue({ status: 'ok', data: answer })
    render(
      <AskBox
        context={{
          currentView: 'agent:writer',
          focusedArtifact: { kind: 'agent', id: 'writer' },
          recentRuns: [],
        }}
        placeholder="Ask anything"
      />
    )
    expect(screen.getByPlaceholderText('Ask anything')).toBeInTheDocument()
    await ask('What is running?')

    expect(mockClient.ask).toHaveBeenCalledWith({
      text: 'What is running?',
      attachments: [],
      context: {
        currentView: 'agent:writer',
        focusedArtifact: { kind: 'agent', id: 'writer' },
        recentRuns: [],
      },
    })
    expect(await screen.findByText(/is still going for/)).toBeInTheDocument()
    expect(screen.getByText('What is running?')).toBeInTheDocument()
  })

  it('opens a source and asks a follow-up', async () => {
    mockClient.ask.mockResolvedValue({ status: 'ok', data: answer })
    render(<AskBox context={HOME} placeholder="Ask" />)
    const user = await ask('Status?')

    await user.click(await screen.findByRole('button', { name: 'Writer' }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'agent',
      agentId: 'writer',
    })

    await user.click(screen.getByRole('button', { name: 'What failed today?' }))
    await waitFor(() => expect(mockClient.ask).toHaveBeenCalledTimes(2))
    expect(mockClient.ask.mock.calls[1]?.[0]).toMatchObject({
      text: 'What failed today?',
    })
  })

  it('shows a clarifying question and errors', async () => {
    mockClient.ask
      .mockResolvedValueOnce({
        status: 'ok',
        data: { kind: 'clarification', data: 'Answer or do it?' },
      })
      .mockResolvedValueOnce({ status: 'error', error: 'Add an API key' })
    render(<AskBox context={HOME} placeholder="Ask" />)

    const user = await ask('the thing')
    expect(await screen.findByText('Answer or do it?')).toBeInTheDocument()

    await user.type(screen.getByRole('textbox'), 'again{Enter}')
    expect(await screen.findByText('Add an API key')).toBeInTheDocument()
  })

  it('dismisses the result', async () => {
    mockClient.ask.mockResolvedValue({ status: 'ok', data: answer })
    render(<AskBox context={HOME} placeholder="Ask" />)
    const user = await ask('Status?')
    await screen.findByText(/is still going for/)

    await user.click(screen.getByRole('button', { name: /dismiss/i }))
    expect(screen.queryByText(/is still going for/)).not.toBeInTheDocument()
  })
})
