import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import type { PendingClarification } from '@/lib/bindings'
import {
  ALLOW_ALWAYS_ANSWER,
  ALLOW_ONCE_ANSWER,
  DENY_ANSWER,
  formatRemaining,
  roundOf,
} from '@/hooks/useClarifications'
import { ClarificationCard } from './ClarificationCard'
import { EscalationApprovalCard } from './EscalationApprovalCard'

const { mockClient } = vi.hoisted(() => ({
  mockClient: {
    respondToClarification: vi.fn(),
    escalateClarification: vi.fn(),
    getPendingClarifications: vi.fn(),
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

const NOW = 1_000_000

function pending(
  overrides: Partial<PendingClarification> = {}
): PendingClarification {
  return {
    taskId: 'task-1',
    agentId: 'writer',
    question: {
      id: 'r2-q1',
      question: 'How much may I spend?',
      topic: 'budget',
      requiresHuman: false,
    },
    deadline: NOW + 125_000,
    approvalId: 'approval-1',
    ...overrides,
  }
}

const now = () => NOW

describe('clarification helpers', () => {
  it('reads the round off a question ID', () => {
    expect(roundOf('r2-q1')).toBe(2)
    expect(roundOf('r10-q3')).toBe(10)
    expect(roundOf('q1')).toBeNull()
  })

  it('formats the time left', () => {
    expect(formatRemaining(125_000)).toBe('2:05')
    expect(formatRemaining(500)).toBe('0:01')
    expect(formatRemaining(3_725_000)).toBe('1:02:05')
  })
})

describe('ClarificationCard', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockClient.escalateClarification.mockResolvedValue({
      status: 'ok',
      data: null,
    })
  })

  it('shows the question, the round, and that a person must answer', () => {
    render(<ClarificationCard clarification={pending()} maxRounds={3} />)
    expect(screen.getByTestId('clarification-question')).toHaveTextContent(
      'How much may I spend?'
    )
    expect(screen.getByText('Round 2/3')).toBeInTheDocument()
    expect(screen.getByText('Waiting for human…')).toBeInTheDocument()
    expect(screen.queryByTestId('clarification-answer')).toBeNull()
  })

  it('shows the answer once it came back', () => {
    render(
      <ClarificationCard clarification={pending()} answer="Up to 20 EUR" />
    )
    expect(screen.getByTestId('clarification-answer')).toHaveTextContent(
      'Up to 20 EUR'
    )
    expect(screen.queryByText('Waiting for human…')).toBeNull()
  })

  it('lets the user take over a question the agent is answering', async () => {
    const user = userEvent.setup()
    render(<ClarificationCard clarification={pending({ approvalId: null })} />)
    expect(screen.getByText('The agent is answering')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Ask me instead' }))
    await waitFor(() =>
      expect(mockClient.escalateClarification).toHaveBeenCalledWith(
        'task-1',
        'r2-q1'
      )
    )
  })
})

describe('EscalationApprovalCard', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockClient.respondToClarification.mockResolvedValue({
      status: 'ok',
      data: null,
    })
  })

  it('shows the question with its context and deadline', () => {
    render(
      <EscalationApprovalCard
        clarification={pending()}
        capability="summarize"
        provider="papers"
        now={now}
      />
    )
    expect(screen.getByText('How much may I spend?')).toBeInTheDocument()
    expect(screen.getByText('task-1')).toBeInTheDocument()
    expect(screen.getByText('summarize')).toBeInTheDocument()
    expect(screen.getByText('papers')).toBeInTheDocument()
    expect(screen.getByText('2:05 left')).toBeInTheDocument()
  })

  it('sends a typed answer for the round', async () => {
    const user = userEvent.setup()
    render(<EscalationApprovalCard clarification={pending()} now={now} />)
    await user.click(screen.getByRole('button', { name: 'Answer' }))
    await user.type(screen.getByLabelText('Your answer'), '  Up to 20 EUR ')
    await user.click(screen.getByRole('button', { name: 'Send' }))
    await waitFor(() =>
      expect(mockClient.respondToClarification).toHaveBeenCalledWith({
        taskId: 'task-1',
        round: 2,
        answers: { 'r2-q1': 'Up to 20 EUR' },
      })
    )
  })

  it.each([
    ['Allow once', ALLOW_ONCE_ANSWER],
    ['Allow always', ALLOW_ALWAYS_ANSWER],
    ['Deny', DENY_ANSWER],
  ])('%s sends its decision', async (button, answer) => {
    const user = userEvent.setup()
    render(<EscalationApprovalCard clarification={pending()} now={now} />)
    await user.click(screen.getByRole('button', { name: button }))
    await waitFor(() =>
      expect(mockClient.respondToClarification).toHaveBeenCalledWith({
        taskId: 'task-1',
        round: 2,
        answers: { 'r2-q1': answer },
      })
    )
  })

  it('offers no agent answer when only a person can answer', () => {
    render(
      <EscalationApprovalCard
        clarification={pending({ agentId: '_user' })}
        now={now}
      />
    )
    expect(screen.queryByRole('button', { name: 'Allow once' })).toBeNull()
    expect(screen.getByRole('button', { name: 'Deny' })).toBeInTheDocument()
  })

  it('shows why an answer was refused', async () => {
    mockClient.respondToClarification.mockResolvedValue({
      status: 'error',
      error: 'This round of questions is already over',
    })
    const user = userEvent.setup()
    render(<EscalationApprovalCard clarification={pending()} now={now} />)
    await user.click(screen.getByRole('button', { name: 'Deny' }))
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'This round of questions is already over'
    )
  })

  it('disables the decisions once the deadline passed', () => {
    render(
      <EscalationApprovalCard
        clarification={pending({ deadline: NOW - 1 })}
        now={now}
      />
    )
    expect(screen.getByText('Deadline passed')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Deny' })).toBeDisabled()
  })
})
