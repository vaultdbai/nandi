import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type {
  CompanionSnapshot,
  ConversationSnapshot,
  WorkItem,
} from '@/lib/bindings'
import { request as requestFixture } from '@/components/requests/request-fixtures'
import { localDayStart } from '@/hooks/useCompanion'
import { useAgentStore } from '@/store/agent-store'
import { useCompanionStore } from '@/store/companion-store'
import { useNavigationStore } from '@/store/navigation-store'
import { CompanionApp } from './CompanionWindow'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })
const mocks = () =>
  commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const NOW = Date.now()
const MIN = 60_000

function snapshot(
  overrides: Partial<CompanionSnapshot> = {}
): CompanionSnapshot {
  return {
    running: 0,
    waiting: 0,
    completedToday: 5,
    failedToday: 0,
    health: 'healthy',
    pendingApprovals: 0,
    pinned: false,
    conversationId: null,
    blocker: null,
    notify: true,
    notifyResults: true,
    scopeRequestId: null,
    bubble: true,
    startBisonOnly: false,
    ...overrides,
  }
}

const approval = {
  id: 'ap1',
  agentId: 'writer',
  runId: 'r1',
  conversationId: 'agent:writer',
  tool: 'shell',
  args: { cmd: 'git push origin main' },
  createdAt: NOW - 2 * MIN,
  expiresAt: NOW + 12 * MIN,
}

function work(overrides: Partial<WorkItem>): WorkItem {
  return {
    id: 'w1',
    kind: 'workflow',
    title: 'Daily digest',
    state: 'active',
    owner: null,
    progress: { completedSteps: 1, totalSteps: 4, percent: 25 },
    nextAction: null,
    workflowId: 'digest',
    rating: null,
    startedAt: NOW - 10 * MIN,
    updatedAt: NOW - MIN,
    ...overrides,
  }
}

function conversation(content: string): ConversationSnapshot {
  return {
    conversationId: 'c1',
    messages: [
      {
        kind: 'user',
        id: 'm1',
        runId: 'r1',
        createdAt: 1,
        content: 'What is stuck?',
        attachments: [],
      },
      { kind: 'agent', id: 'm2', runId: 'r1', createdAt: 2, content },
    ],
    currentRun: null,
  }
}

beforeEach(() => {
  useAgentStore.setState(useAgentStore.getInitialState())
  useNavigationStore.setState(useNavigationStore.getInitialState())
  useCompanionStore.setState({ focusedRequestId: null })
  Object.assign(commands, {
    getCompanionSnapshot: ok(snapshot()),
    sendCompanionMessage: ok({
      runId: 'r9',
      messageId: 'm9',
      conversationTitle: null,
      queued: false,
    }),
    setCompanionPinned: ok(null),
    setCompanionNotify: ok(null),
    getDefaultModel: ok({
      defaultModel: 'claude:claude-sonnet-5',
      effectiveModel: 'claude:claude-sonnet-5',
    }),
    getConversation: ok(conversation('')),
    listWork: ok([]),
    listAllRuns: ok({ rows: [], total: 0 }),
    listDelegationHistory: ok([]),
    listPendingApprovals: ok([]),
    listPendingManualSteps: ok([]),
    getPendingClarifications: ok([]),
    listDismissedWork: ok([]),
    listGatewayHolds: ok([]),
    approveAction: ok(null),
    listAgents: ok([
      {
        id: 'writer',
        name: 'Report Writer',
        description: '',
        skillPath: '',
        status: 'idle',
        currentTask: null,
        origin: { kind: 'local' },
        builtin: false,
      },
    ]),
    getRequest: ok(
      requestFixture({ id: 'task-1', title: 'Summarize Q3 risk' })
    ),
  })
})

async function expand(user: ReturnType<typeof userEvent.setup>) {
  await user.click(await screen.findByRole('button', { name: /companion/i }))
}

describe('the orb', () => {
  it('counts what needs you, like the Home badge', async () => {
    Object.assign(commands, { listPendingApprovals: ok([approval]) })
    render(<CompanionApp docked />)
    const orb = await screen.findByRole('button', { name: /companion/i })
    await waitFor(() => expect(orb).toHaveAttribute('data-state', 'needsYou'))
    expect(screen.getByTestId('orb-badge')).toHaveTextContent('1')
    expect(commands.getCompanionSnapshot).toHaveBeenCalledWith(localDayStart())
  })

  it('pulses blue while work runs and nothing needs you', async () => {
    Object.assign(commands, { listWork: ok([work({})]) })
    render(<CompanionApp docked />)
    const orb = await screen.findByRole('button', { name: /companion/i })
    await waitFor(() => expect(orb).toHaveAttribute('data-state', 'running'))
    expect(screen.queryByTestId('orb-badge')).toBeNull()
  })

  it('turns red with no model, and Add a model is the only thing to do', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getCompanionSnapshot: ok(snapshot({ blocker: 'noModel' })),
      getDefaultModel: ok({
        defaultModel: 'claude:claude-sonnet-5',
        effectiveModel: null,
      }),
    })
    render(<CompanionApp docked />)
    const orb = await screen.findByRole('button', { name: /companion/i })
    await waitFor(() => expect(orb).toHaveAttribute('data-state', 'problem'))
    expect(screen.getByTestId('orb-badge')).toHaveTextContent('!')

    await user.click(orb)
    expect(
      await screen.findByRole('button', { name: 'Connect a model' })
    ).toBeInTheDocument()
    // The ask box says why it won't send
    expect(
      screen.getByPlaceholderText(/Add a model to ask/)
    ).toBeInTheDocument()
  })
})

describe('the card', () => {
  it("is Home in miniature: Needs you, Running and Recent with Home's rows", async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      listPendingApprovals: ok([approval]),
      listWork: ok([
        work({}),
        work({
          id: 'w2',
          title: 'Sync CRM',
          state: 'failed',
          updatedAt: NOW - 5 * MIN,
        }),
      ]),
    })
    render(<CompanionApp docked />)
    await expand(user)

    const needsYou = await screen.findByRole('region', { name: /Needs you/ })
    expect(
      within(needsYou).getByRole('button', { name: 'Approve' })
    ).toBeInTheDocument()
    const running = screen.getByRole('region', { name: /Running/ })
    expect(within(running).getByText('Daily digest')).toBeInTheDocument()
    expect(within(running).getByText('step 2 of 4')).toBeInTheDocument()
    expect(screen.getByRole('region', { name: 'Recent' })).toHaveTextContent(
      'Sync CRM'
    )
    expect(screen.getByTestId('companion-summary')).toHaveTextContent(
      '1 needs you · 1 running · 5 done today'
    )
    // One assistant: no agent picker
    expect(screen.queryByRole('button', { name: /assistant/i })).toBeNull()
  })

  it('approves inline without opening Nandi', async () => {
    const user = userEvent.setup()
    Object.assign(commands, { listPendingApprovals: ok([approval]) })
    render(<CompanionApp docked />)
    await expand(user)
    await user.click(await screen.findByRole('button', { name: 'Approve' }))
    await waitFor(() =>
      expect(mocks().approveAction).toHaveBeenCalledWith(
        'writer',
        'ap1',
        'approved'
      )
    )
    expect(useNavigationStore.getState().current).toEqual({ kind: 'home' })
  })

  it('shows at most three things that need you, then "+ N more"', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      listPendingApprovals: ok(
        [1, 2, 3, 4, 5].map(n => ({ ...approval, id: `ap${n}` }))
      ),
    })
    render(<CompanionApp docked />)
    await expand(user)
    const needsYou = await screen.findByRole('region', { name: /Needs you/ })
    expect(
      within(needsYou).getAllByRole('button', { name: 'Approve' })
    ).toHaveLength(3)
    await user.click(within(needsYou).getByRole('button', { name: '+ 2 more' }))
    expect(useNavigationStore.getState().current).toEqual({ kind: 'home' })
  })

  it('pins a watched Request at the top of Running', async () => {
    const user = userEvent.setup()
    useCompanionStore.getState().focusRequest('task-1')
    render(<CompanionApp docked />)
    // Watching opens the card
    expect(await screen.findByTestId('request-card')).toHaveTextContent(
      'Summarize Q3 risk'
    )
    await user.click(screen.getByRole('button', { name: 'Stop watching' }))
    expect(useCompanionStore.getState().focusedRequestId).toBeNull()
  })

  it('turns "Notify me" off from its menu', async () => {
    const user = userEvent.setup()
    render(<CompanionApp docked />)
    await expand(user)
    await user.click(await screen.findByRole('button', { name: 'More' }))
    await user.click(
      await screen.findByRole('menuitemcheckbox', { name: 'Notify me' })
    )
    await waitFor(() =>
      expect(mocks().setCompanionNotify).toHaveBeenCalledWith(false)
    )
  })
})

describe('the conversation', () => {
  it('sends to the companion conversation from one input', async () => {
    const user = userEvent.setup()
    render(<CompanionApp docked />)
    await expand(user)
    // No Chat | Ask toggle
    expect(screen.queryByRole('radio', { name: 'Ask' })).toBeNull()
    await user.type(
      await screen.findByPlaceholderText(/Ask Nandi/),
      'What did my agents do today?{Enter}'
    )
    await waitFor(() =>
      expect(mocks().sendCompanionMessage).toHaveBeenCalledWith(
        'What did my agents do today?',
        null,
        null
      )
    )
  })

  it('cites Requests as live cards and scopes the chat to one', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getCompanionSnapshot: ok(snapshot({ conversationId: 'c1' })),
      getConversation: ok(
        conversation(
          'It waits on you: [Summarize Q3 risk](nandi://request/task-1)'
        )
      ),
    })
    render(<CompanionApp docked />)
    await expand(user)
    const card = await screen.findByTestId('request-card')
    expect(card).toHaveTextContent('Summarize Q3 risk')

    await user.click(
      within(card).getByRole('button', { name: /Ask about Summarize Q3 risk/ })
    )
    expect(await screen.findByTestId('scope-chip')).toHaveTextContent(
      'About: Summarize Q3 risk'
    )
    await user.type(
      screen.getByPlaceholderText(/Ask Nandi/),
      'Retry it with Sam{Enter}'
    )
    await waitFor(() =>
      expect(mocks().sendCompanionMessage).toHaveBeenCalledWith(
        'Retry it with Sam',
        'c1',
        'task-1'
      )
    )

    // Clearing the chip goes back to the whole Nandi
    await user.click(screen.getByRole('button', { name: 'Clear' }))
    expect(screen.queryByTestId('scope-chip')).toBeNull()
  })

  it('opens the conversation in Studio', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getCompanionSnapshot: ok(snapshot({ conversationId: 'c1' })),
    })
    render(<CompanionApp docked />)
    await expand(user)
    await user.click(await screen.findByRole('button', { name: 'More' }))
    await user.click(
      await screen.findByRole('menuitem', { name: 'Open in Ask' })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'studio',
      conversationId: 'c1',
    })
  })
})

describe('companion helpers', () => {
  it('computes local midnight', () => {
    const start = new Date(localDayStart())
    expect([start.getHours(), start.getMinutes(), start.getSeconds()]).toEqual([
      0, 0, 0,
    ])
  })
})
