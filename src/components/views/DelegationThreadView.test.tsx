import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { networkSnapshot, taskFixture } from '@/components/agents/task-fixtures'
import { useNavigationStore } from '@/store/navigation-store'
import {
  buildThread,
  extractUsage,
} from '@/components/agents/delegation-thread-model'
import { DelegationThreadView } from './DelegationThreadView'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

const base = 1_700_000_000_000

const completed = taskFixture({
  state: 'completed',
  createdAt: base,
  acknowledgedAt: base + 1_000,
  completedAt: base + 90_000,
  result: {
    papers: ['A'],
    usage: { inputTokens: 1000, outputTokens: 500 },
    costUsd: 0.25,
  },
})

const events = [
  {
    taskId: 'task-1',
    fromState: null,
    toState: 'submitted',
    reason: null,
    at: base + 10,
  },
  {
    taskId: 'task-1',
    fromState: 'submitted',
    toState: 'working',
    reason: null,
    at: base + 1_000,
  },
  {
    taskId: 'task-1',
    fromState: 'working',
    toState: 'completed',
    reason: 'done',
    at: base + 90_000,
  },
]

const messages = [
  {
    id: 1,
    kind: 'progress',
    payload: { percent: 30, note: 'Searching' },
    at: base + 2_000,
  },
  { id: 2, kind: 'status_query', payload: null, at: base + 3_000 },
  {
    id: 3,
    kind: 'status_response',
    payload: { state: 'working', etaSeconds: 30 },
    at: base + 3_500,
  },
  {
    id: 4,
    kind: 'clarification_request',
    payload: {
      taskId: 'task-1',
      round: 1,
      questions: [
        {
          id: 'q1',
          question: 'Which years?',
          topic: 'dates',
          requiresHuman: false,
        },
      ],
    },
    at: base + 4_000,
  },
  {
    id: 5,
    kind: 'clarification_response',
    payload: { taskId: 'task-1', round: 1, answers: { q1: '2020 to 2024' } },
    at: base + 5_000,
  },
  {
    id: 6,
    kind: 'escalation',
    payload: { question: 'Spend over budget?' },
    at: base + 6_000,
  },
  { id: 7, kind: 'note', payload: 'Used cached index', at: base + 7_000 },
  { id: 8, kind: 'custom_kind', payload: { x: 1 }, at: base + 8_000 },
]

describe('DelegationThreadView', () => {
  beforeEach(() => {
    Object.assign(commands, {
      getTask: ok(completed),
      getTaskEvents: ok(events),
      getTaskMessages: ok(messages),
      getPendingClarifications: ok([]),
      getClarificationPolicy: ok({
        allow: true,
        maxRounds: 4,
        autoTopics: [],
        escalateTopics: [],
        timeoutSeconds: 60,
      }),
    })
    useNavigationStore.setState({ current: { kind: 'home' }, history: [] })
  })

  it('renders the header and every message kind in order', async () => {
    render(<DelegationThreadView taskId="task-1" />)

    expect(
      await screen.findByText('Delegation: find-papers')
    ).toBeInTheDocument()
    expect(screen.getByText('writer → papers')).toBeInTheDocument()
    expect(screen.getByText('Duration 1m 30s')).toBeInTheDocument()
    expect(screen.getByText('1.5K tokens')).toBeInTheDocument()

    const thread = await screen.findByRole('list', { name: 'Task thread' })
    await within(thread).findByTestId('thread-progress')
    const ids = [...thread.querySelectorAll('[data-testid]')].map(el =>
      el.getAttribute('data-testid')
    )
    expect(ids).toEqual([
      'thread-delegate',
      'thread-state',
      'thread-state',
      'thread-progress',
      'thread-status-query',
      'thread-status-response',
      'thread-clarification-request',
      'thread-clarification-response',
      'thread-escalation',
      'thread-note',
      'thread-other',
      'thread-state',
      'thread-result',
    ])
    expect(screen.getByText('Progress: 30%')).toBeInTheDocument()
    expect(
      screen.getByText('Questions from the provider (round 1)')
    ).toBeInTheDocument()
    // The answer is paired with its question's text
    const answers = screen.getByTestId('thread-clarification-response')
    expect(answers).toHaveTextContent('Which years?')
    expect(answers).toHaveTextContent('2020 to 2024')
    expect(screen.getByText('Spend over budget?')).toBeInTheDocument()
    expect(screen.getByText('Used cached index')).toBeInTheDocument()
    expect(screen.getByTestId('thread-result')).toHaveTextContent('"papers"')
  })

  it("shows why the relay picked the provider, in the user's words", async () => {
    Object.assign(commands, {
      getTaskMessages: ok([
        {
          id: 1,
          kind: 'route',
          payload: {
            provider: 'Bob',
            reasons: [
              { code: 'covers', name: 'Bob', awayName: 'Alice' },
              { code: 'busy', name: 'Bob', load: 3, max: 3, waitMinutes: 20 },
            ],
            redirectedFrom: 'alice-1',
          },
          at: base + 1_000,
        },
      ]),
    })
    render(<DelegationThreadView taskId="task-1" />)

    const route = await screen.findByTestId('thread-route')
    expect(route).toHaveTextContent('Why Bob')
    expect(route).toHaveTextContent(
      'Alice is away; Bob covers · Bob 3/3 busy (about 20 min wait)'
    )
  })

  it('links to the provider profile and the requester agent', async () => {
    const user = userEvent.setup()
    render(<DelegationThreadView taskId="task-1" />)

    await user.click(
      await screen.findByRole('button', { name: 'View member profile' })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'member-profile',
      instanceId: 'peer',
    })

    await user.click(
      screen.getByRole('button', { name: 'Re-run with modified policy' })
    )
    // The question policy lives in the agent's Settings
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'agent',
      agentId: 'writer',
      tab: 'settings',
    })
  })

  it('says who reassigned the task and opens the new attempt', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getTask: ok(taskFixture({ state: 'reassigned', createdAt: base })),
      getTaskEvents: ok([]),
      getTaskMessages: ok([
        {
          id: 1,
          kind: 'reassigned',
          payload: {
            taskId: 'task-1',
            fromInstance: 'peer',
            toInstance: 'bob-1',
            toName: 'Bob',
            by: 'Alice',
            reason: 'Swamped this week',
            newTaskId: 'task-2',
            reassignedAt: base + 1_000,
          },
          at: base + 1_000,
        },
      ]),
    })
    render(<DelegationThreadView taskId="task-1" />)
    const note = await screen.findByTestId('thread-reassigned')
    expect(note).toHaveTextContent('Reassigned to Bob by Alice')
    expect(note).toHaveTextContent('Swamped this week')
    await user.click(
      within(note).getByRole('button', { name: 'Open the new attempt' })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'delegation-thread',
      taskId: 'task-2',
    })
  })

  it("summarizes the local requester agent's own policy", async () => {
    render(<DelegationThreadView taskId="task-1" />)
    expect(
      await screen.findByTestId('delegation-thread-policy')
    ).toBeInTheDocument()
    await screen.findByText('Up to 4 round(s) of questions, 60s each')
    expect(commands.getClarificationPolicy).toHaveBeenCalledWith('writer')
  })

  describe("the user's own request to a member's gateway", () => {
    const asked = taskFixture({
      requesterAgent: '_user',
      providerAgent: 'gateway',
      capability: 'gateway',
      state: 'inputRequired',
      createdAt: base,
      acknowledgedAt: base + 1_000,
    })
    const pendingForUser = {
      taskId: 'task-1',
      agentId: '_user',
      question: {
        id: 'r1-q1',
        question: 'Which city?',
        topic: 'location',
        requiresHuman: false,
      },
      deadline: null,
      approvalId: null,
    }

    beforeEach(() => {
      Object.assign(commands, {
        getTask: ok(asked),
        getTaskEvents: ok([
          {
            taskId: 'task-1',
            fromState: 'delivered',
            toState: 'acknowledged',
            reason: "Routed to Bob's Helper: General knowledge question",
            at: base + 1_000,
          },
        ]),
        getTaskMessages: ok([]),
        getPendingClarifications: ok([pendingForUser]),
        getClarificationPolicy: vi.fn(),
        respondToClarification: ok(null),
      })
    })

    it('names the user, shows the routing reason, and offers no re-run', async () => {
      render(<DelegationThreadView taskId="task-1" />)
      expect(await screen.findByText('You → gateway')).toBeInTheDocument()
      expect(
        screen.getByText('You asked gateway to gateway')
      ).toBeInTheDocument()
      expect(
        await screen.findByText(/Routed to Bob's Helper/)
      ).toBeInTheDocument()
      expect(
        screen.getByRole('button', { name: 'Re-run with modified policy' })
      ).toBeDisabled()
      expect(
        screen.getByText(
          'You sent this yourself, so every question comes to you.'
        )
      ).toBeInTheDocument()
      expect(commands.getClarificationPolicy).not.toHaveBeenCalled()
    })

    it('lets the user answer a question asked of them', async () => {
      const user = userEvent.setup()
      render(<DelegationThreadView taskId="task-1" />)
      expect(await screen.findByText('Which city?')).toBeInTheDocument()
      // Their own question: no agent to wait on or hand it to
      expect(
        screen.queryByText('The agent is answering')
      ).not.toBeInTheDocument()
      expect(
        screen.queryByRole('button', { name: 'Allow once' })
      ).not.toBeInTheDocument()

      await user.click(screen.getByRole('button', { name: 'Answer' }))
      await user.type(screen.getByLabelText('Your answer'), 'Boston')
      await user.click(screen.getByRole('button', { name: 'Send' }))
      await waitFor(() =>
        expect(commands.respondToClarification).toHaveBeenCalledWith({
          taskId: 'task-1',
          round: 1,
          answers: { 'r1-q1': 'Boston' },
        })
      )
    })
  })

  it('says a request is queued while the member is offline', async () => {
    Object.assign(commands, {
      getTask: ok(taskFixture({ state: 'submitted', acknowledgedAt: null })),
      getTaskEvents: ok([]),
      getTaskMessages: ok([]),
      getNetworkSnapshot: ok(networkSnapshot(false)),
    })
    render(<DelegationThreadView taskId="task-1" />)
    expect(
      await screen.findByTestId('thread-queued-offline')
    ).toHaveTextContent(
      "Peer Person is offline. The request is queued and will be sent automatically when they're back."
    )
  })

  it('shows the error when the task cannot load', async () => {
    Object.assign(commands, {
      getTask: vi
        .fn()
        .mockResolvedValue({ status: 'error', error: 'no such task' }),
    })
    render(<DelegationThreadView taskId="task-1" />)
    expect(await screen.findByRole('alert')).toHaveTextContent('no such task')
  })
})

describe('thread model', () => {
  it('omits the result while the task runs', () => {
    const items = buildThread(taskFixture(), [], [])
    expect(items.map(i => i.kind)).toEqual(['delegate'])
  })

  it('reads usage from common result shapes', () => {
    expect(extractUsage(null)).toEqual({ tokens: null, costUsd: null })
    expect(extractUsage({ totalTokens: 42, cost: 1 })).toEqual({
      tokens: 42,
      costUsd: 1,
    })
  })
})
