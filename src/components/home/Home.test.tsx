import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type {
  AgentHealth,
  RelayStatus,
  RemoteStatus,
  WorkItem,
} from '@/lib/bindings'
import {
  agentInfo,
  dashboardSnapshot,
  packManifest,
  run,
} from '@/test/fixtures'
import { emitMockEvent } from '@/test/mock-events'
import { useAgentStore } from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'
import { AllActivityView } from './AllActivityView'
import { HomeView } from './HomeView'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })
const mocks = () =>
  commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const NOW = Date.now()
const MIN = 60_000

const relay = (connected: boolean): RelayStatus => ({
  connected,
  state: connected ? 'connected' : 'disconnected',
  reason: null,
  instanceId: 'me',
  relayUrl: connected ? 'wss://relay.nandi.dev/relay' : null,
  relayUrlSource: connected ? 'config' : null,
  agentsOnline: 0,
  instancesOnline: connected ? 5 : 0,
  lastConnectedAt: null,
  lastHeartbeatAt: null,
})

const health = (id: string): AgentHealth => ({
  agent: agentInfo(id, id),
  lastRunAt: null,
  successRateWeek: null,
  avgDurationMs: null,
})

function item(overrides: Partial<WorkItem>): WorkItem {
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

/** Something has happened: the full Home, not Get started */
const busy = () =>
  useAgentStore.setState({
    dashboard: dashboardSnapshot({ agentHealth: [health('writer')] }),
  })

beforeEach(() => {
  localStorage.clear()
  useAgentStore.setState({ dashboard: dashboardSnapshot() })
  useNavigationStore.setState(useNavigationStore.getInitialState())
  Object.assign(commands, {
    listWork: ok([]),
    listAllRuns: ok({ rows: [], total: 0 }),
    listDelegationHistory: ok([]),
    listPendingApprovals: ok([]),
    listPendingManualSteps: ok([]),
    getPendingClarifications: ok([]),
    listDismissedWork: ok([]),
    listGatewayHolds: ok([]),
    listGatewayAssignments: ok([]),
    dismissWork: ok(null),
    approveAction: ok(null),
    rateWork: ok(null),
    getRelayStatus: ok(relay(false)),
    listOnlineAgents: ok([]),
    getRemoteStatus: ok({ configured: true } as RemoteStatus),
    getDefaultModel: ok({
      defaultModel: 'claude:claude-sonnet-5',
      effectiveModel: 'claude:claude-sonnet-5',
    }),
  })
})

describe('Home, first run', () => {
  it('shows Get started only, with no zero tiles', async () => {
    render(<HomeView />)
    expect(await screen.findByText('Get started')).toBeInTheDocument()
    expect(screen.getByTestId('get-started-packs')).toBeInTheDocument()
    expect(screen.getByTestId('get-started-studio')).toBeInTheDocument()
    expect(screen.getByTestId('get-started-network')).toBeInTheDocument()
    expect(screen.queryByTestId(/home-tile-/)).toBeNull()
    expect(screen.queryByText(/Recent/)).toBeNull()
    expect(screen.queryByText(/New Agent/)).toBeNull()
  })

  it('sends each Get started card to its page', async () => {
    const user = userEvent.setup()
    render(<HomeView />)
    const current = () => useNavigationStore.getState().current
    await user.click(
      await screen.findByRole('button', { name: 'Browse packs' })
    )
    expect(current()).toEqual({ kind: 'starter-packs' })
    await user.click(screen.getByRole('button', { name: 'Open Ask' }))
    expect(current()).toEqual({ kind: 'studio', conversationId: null })
    await user.click(screen.getByRole('button', { name: 'Set up network' }))
    expect(current()).toEqual({ kind: 'network', tab: 'connection' })
  })

  it('offers to connect a model on the Models pane, once', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getDefaultModel: ok({
        defaultModel: 'claude:claude-sonnet-5',
        effectiveModel: null,
      }),
    })
    render(<HomeView />)
    await screen.findByText('Get started')
    const buttons = await screen.findAllByRole('button', {
      name: 'Connect a model',
    })
    // The banner, not also an Inbox row
    expect(buttons).toHaveLength(1)
    await user.click(buttons[0] as HTMLElement)
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'settings',
      section: 'models',
    })
  })

  it('welcomes the packs just installed with something to try', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      listMarketplacePacks: ok([
        packManifest('research-pack', 'Research', {
          suggestedAsks: ['Summarize my reading list'],
        }),
      ]),
    })
    render(<HomeView welcome={{ packIds: ['research-pack'] }} />)
    expect(
      await screen.findByText("You're set up. 1 agent installed from Research.")
    ).toBeInTheDocument()
    await user.click(
      screen.getByRole('button', { name: 'Summarize my reading list' })
    )
    await waitFor(() =>
      expect(
        vi.mocked(commands.sendStudioMessage).mock.calls[0]?.slice(0, 2)
      ).toEqual(['studio-new', 'Summarize my reading list'])
    )
  })

  it('asks Nandi from the top of Home', async () => {
    const user = userEvent.setup()
    render(<HomeView />)
    await user.type(
      await screen.findByRole('textbox', { name: 'Ask Nandi' }),
      'What failed today?{Enter}'
    )
    await waitFor(() =>
      expect(
        vi.mocked(commands.sendStudioMessage).mock.calls.at(-1)?.slice(0, 2)
      ).toEqual(['studio-new', 'What failed today?'])
    )
  })
})

describe('Home, with activity', () => {
  beforeEach(busy)

  it('says when nothing needs you, and links to All activity', async () => {
    const user = userEvent.setup()
    render(<HomeView />)
    expect(
      await screen.findByText('Nothing needs you. Nothing is running.')
    ).toBeInTheDocument()
    // No relay: one line, never "Relay unavailable"
    expect(screen.getByTestId('home-network')).toHaveTextContent(
      'Not connected.'
    )
    expect(screen.queryByText(/unavailable/i)).toBeNull()
    await user.click(screen.getByRole('button', { name: /All activity/ }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'home-activity',
    })
  })

  it('names held work and questions from other people in plain words', async () => {
    const hold = (taskId: string, capability: string) => ({
      taskId,
      callerInstance: 'i-dev',
      callerName: 'Dev Chechi',
      capability,
      summary: 'Please confirm the walkthrough time',
      status: 'pending' as const,
      createdAt: Date.now(),
      expiresAt: Date.now() + 600_000,
    })
    Object.assign(commands, {
      listGatewayHolds: ok([hold('t1', 'assign-me'), hold('t2', 'ask-me')]),
    })
    render(<HomeView />)
    expect(
      await screen.findByText('Dev Chechi asked you to take on some work')
    ).toBeInTheDocument()
    expect(
      screen.getByText('Dev Chechi asked you a question')
    ).toBeInTheDocument()
    expect(screen.queryByText(/asked for assign-me/)).toBeNull()
  })

  it('shows work assigned to you as one line, opening its chat', async () => {
    const user = userEvent.setup()
    const assignment = {
      taskId: 't9',
      callerInstance: 'i-dev',
      callerName: 'Dev Chechi',
      request: 'Send the SOC 1 evidence owners',
      status: 'open' as const,
      note: null,
      createdAt: Date.now() - 3_600_000,
      dueAt: Date.now() - 2 * 3_600_000,
      closedAt: null,
      links: [],
    }
    Object.assign(commands, {
      listGatewayAssignments: ok([assignment]),
    })
    render(<HomeView />)
    const inbox = await screen.findByRole('region', { name: /Inbox/ })
    const row = await within(inbox).findByRole('button', {
      name: /Dev Chechi assigned you this/,
    })
    expect(row).toHaveTextContent(/Overdue by/)
    // Calm: no inline note or buttons in the list
    expect(within(inbox).queryByRole('textbox')).toBeNull()
    await user.click(row)
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'inbox-thread',
      conversationId: 'inbox:assignment:t9',
    })
  })

  it('shows running work with its step, and opens it', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      listWork: ok([item({})]),
      getRelayStatus: ok(relay(true)),
    })
    render(<HomeView />)
    const running = await screen.findByRole('region', { name: 'Running · 1' })
    expect(running).toHaveTextContent('Daily digest')
    expect(running).toHaveTextContent('step 2 of 4')
    expect(await screen.findByTestId('home-network')).toHaveTextContent(
      'Connected · 4 people online'
    )
    await user.click(
      within(running).getByRole('button', { name: /Daily digest/ })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'work-item',
      id: 'w1',
    })
  })

  it('lists recent results; a delegation can be rated, a run opens', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      listWork: ok([
        item({
          id: 't1',
          kind: 'task',
          title: 'Translate the contract',
          state: 'done',
          owner: 'Priya',
          updatedAt: NOW - MIN,
        }),
      ]),
      listAllRuns: ok({
        rows: [
          run('r9', 'writer', {
            status: 'completed',
            input: 'Draft the weekly email',
            startedAt: NOW - 5 * MIN,
            endedAt: NOW - 4 * MIN,
          }),
        ],
        total: 1,
      }),
    })
    render(<HomeView />)
    const recent = await screen.findByRole('region', { name: 'Recent' })
    await within(recent).findByText('Draft the weekly email')
    expect(recent).toHaveTextContent('To Priya')
    await user.click(within(recent).getByRole('button', { name: '4 out of 5' }))
    await waitFor(() => expect(mocks().rateWork).toHaveBeenCalledWith('t1', 4))
    await user.click(
      within(recent).getByRole('button', { name: /Draft the weekly email/ })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'run-detail',
      runId: 'r9',
    })
  })

  it('asks before rerunning a failed workflow', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      listWork: ok([item({ state: 'failed', updatedAt: NOW - MIN })]),
      runWorkflow: ok('run-2'),
    })
    render(<HomeView />)
    const recent = await screen.findByRole('region', { name: 'Recent' })
    await user.click(
      await within(recent).findByRole('button', { name: 'Retry' })
    )
    expect(
      await screen.findByText('Run "Daily digest" again?')
    ).toBeInTheDocument()
    expect(mocks().runWorkflow).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: 'Run again' }))
    await waitFor(() =>
      expect(mocks().runWorkflow).toHaveBeenCalledWith('digest')
    )
  })

  it('shows at most three tiles, linked to filtered activity', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      listAllRuns: ok({
        rows: [
          run('a', 'writer', {
            status: 'failed',
            startedAt: NOW - MIN,
            endedAt: NOW,
          }),
        ],
        total: 1,
      }),
    })
    render(<HomeView />)
    const failed = await screen.findByTestId('home-tile-failed')
    expect(failed).toHaveTextContent('1')
    expect(screen.getAllByTestId(/home-tile-/)).toHaveLength(3)
    await user.click(failed)
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'home-activity',
      filters: { range: 'today', status: 'failed' },
    })
  })
})

describe('Home, Inbox', () => {
  beforeEach(busy)

  it('shows a tool approval as one line, opening its chat', async () => {
    const user = userEvent.setup()
    Object.assign(commands, { listPendingApprovals: ok([approval]) })
    render(<HomeView />)
    const inbox = await screen.findByRole('region', { name: /Inbox/ })
    const row = await within(inbox).findByRole('button', {
      name: /Run a command: git push origin main/,
    })
    expect(row).toHaveTextContent(/Expires in 1[12]m/)
    await user.click(row)
    expect(mocks().approveAction).not.toHaveBeenCalled()
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'inbox-thread',
      conversationId: 'inbox:approval:ap1',
    })
  })

  it('drops an approval decided elsewhere on the next event', async () => {
    Object.assign(commands, { listPendingApprovals: ok([approval]) })
    render(<HomeView />)
    await screen.findByText('Run a command: git push origin main')
    Object.assign(commands, { listPendingApprovals: ok([]) })
    act(() => emitMockEvent('agent://approval_resolved', {}))
    await waitFor(() =>
      expect(
        screen.queryByText('Run a command: git push origin main')
      ).not.toBeInTheDocument()
    )
  })

  it('keeps a step waiting for approval out of Running', async () => {
    Object.assign(commands, {
      listWork: ok([
        item({
          id: 'w2',
          title: 'Q4 metrics draft',
          state: 'waiting',
          nextAction: {
            description: "Approve step 'publish' for writer",
            kind: 'approval',
            deadline: null,
          } as WorkItem['nextAction'],
        }),
      ]),
    })
    render(<HomeView />)
    const inbox = await screen.findByRole('region', { name: /Inbox/ })
    expect(inbox).toHaveTextContent('Q4 metrics draft')
    expect(screen.queryByRole('region', { name: /Running/ })).toBeNull()
  })

  it('offers a recent failure, after what waits on you', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      listWork: ok([
        item({
          id: 'run-9',
          title: 'Nightly digest',
          state: 'failed',
          updatedAt: NOW - MIN,
        }),
      ]),
    })
    render(<HomeView />)
    const inbox = await screen.findByRole('region', { name: /Inbox/ })
    await user.click(
      await within(inbox).findByRole('button', {
        name: /Nightly digest failed/,
      })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'inbox-thread',
      conversationId: 'inbox:failed:run-9',
    })
  })
})

describe('All activity', () => {
  beforeEach(() => {
    Object.assign(commands, {
      listAllRuns: ok({
        rows: [
          run('r1', 'writer', {
            status: 'completed',
            input: 'Draft the weekly email',
            startedAt: NOW - 5 * MIN,
            endedAt: NOW - 4 * MIN,
          }),
          run('r2', 'writer', {
            status: 'failed',
            input: 'Sync CRM',
            sourceType: 'webhook',
            startedAt: NOW - 3 * MIN,
            endedAt: NOW - 2 * MIN,
          }),
        ],
        total: 2,
      }),
      listDelegationHistory: ok([
        {
          taskId: 't1',
          providerAgent: 'me/translator',
          output: null,
          status: 'completed',
          durationMs: 1,
          tokensIn: 1,
          tokensOut: 1,
          error: null,
          direction: 'inbound',
          capability: 'translate',
          requesterAgent: 'sam/bot',
          providerOwner: null,
          requesterOwner: 'Sam',
          input: 'Translate the contract',
          startedAt: NOW - 10 * MIN,
          endedAt: NOW - 8 * MIN,
        },
      ]),
    })
  })

  it('lists runs and delegations together, with a summary', async () => {
    render(<AllActivityView />)
    expect(
      await screen.findByText('Draft the weekly email')
    ).toBeInTheDocument()
    expect(screen.getByText('Translate the contract')).toBeInTheDocument()
    expect(screen.getByTestId('activity-summary')).toHaveTextContent(
      '3 requests · 2 done · 1 failed'
    )
  })

  it('keeps filters in the view (they survive Back), and clears them', async () => {
    const user = userEvent.setup()
    useNavigationStore.setState({
      current: { kind: 'home-activity' },
      history: [{ kind: 'home' }],
    })
    const { rerender } = render(<AllActivityView />)
    await screen.findByText('Sync CRM')
    await user.selectOptions(
      screen.getByRole('combobox', { name: 'Direction' }),
      'incoming'
    )
    const current = useNavigationStore.getState().current
    expect(current).toEqual({
      kind: 'home-activity',
      filters: { direction: 'incoming' },
    })
    // No Back step per filter change
    expect(useNavigationStore.getState().history).toEqual([{ kind: 'home' }])
    rerender(
      <AllActivityView
        filters={current.kind === 'home-activity' ? current.filters : {}}
      />
    )
    expect(screen.queryByText('Sync CRM')).toBeNull()
    expect(screen.getByText('Translate the contract')).toBeInTheDocument()
  })

  it('says when nothing matches, with Clear filters', async () => {
    render(<AllActivityView filters={{ search: 'nothing like this' }} />)
    expect(
      await screen.findByText('No activity matches these filters.')
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Export CSV' })).toBeDisabled()
  })

  it('opens a delegation on its Request page', async () => {
    const user = userEvent.setup()
    render(<AllActivityView />)
    await user.click(
      await screen.findByRole('button', { name: 'Translate the contract' })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'request',
      requestId: 't1',
    })
  })
})
