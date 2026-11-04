import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, waitFor, within } from '@/test/test-utils'
import { emitMockEvent } from '@/test/mock-events'
import { commands } from '@/lib/tauri-bindings'
import { networkSnapshot, taskFixture } from '@/components/agents/task-fixtures'
import { useNavigationStore } from '@/store/navigation-store'
import { AgentTaskStrip } from '@/components/agents/AgentTaskStrip'
import { ProviderStatePanel } from './ProviderStatePanel'
import { TaskStatusPanel } from './TaskStatusPanel'

vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })
const mocks = () =>
  commands as unknown as Record<string, ReturnType<typeof vi.fn>>

describe('TaskStatusPanel', () => {
  beforeEach(() => {
    Object.assign(commands, {
      getTask: ok(taskFixture()),
      cancelTask: ok(taskFixture({ state: 'cancelled' })),
      queryTaskStatus: ok({
        taskId: 'task-1',
        state: 'working',
        etaSeconds: 50,
        elapsedSeconds: 12,
        live: true,
      }),
      getTaskMessages: ok([]),
      getPendingClarifications: ok([]),
      listTasks: ok([]),
    })
  })

  it('shows an active task with its provider, state, and ETA', async () => {
    render(<TaskStatusPanel taskId="task-1" />)
    const panel = await screen.findByTestId('task-status-panel')
    expect(panel).toHaveTextContent('Delegating to papers: find-papers')
    expect(panel).toHaveTextContent('Working')
    expect(panel).toHaveTextContent('ETA 1m')
    expect(screen.queryByText(/over twice the ETA/)).not.toBeInTheDocument()
  })

  it('queries status and cancels', async () => {
    const user = userEvent.setup()
    render(<TaskStatusPanel taskId="task-1" />)
    await user.click(
      await screen.findByRole('button', { name: 'Query status' })
    )
    expect(await screen.findByTestId('task-status-report')).toHaveTextContent(
      'Provider says: Working'
    )
    await user.click(screen.getByRole('button', { name: 'Cancel' }))
    await waitFor(() =>
      expect(mocks().cancelTask).toHaveBeenCalledWith('task-1', null)
    )
    // Cancelled is terminal: the panel hides
    await waitFor(() =>
      expect(screen.queryByTestId('task-status-panel')).not.toBeInTheDocument()
    )
  })

  it('shows progress from task://progress', async () => {
    render(<TaskStatusPanel taskId="task-1" />)
    await screen.findByTestId('task-status-panel')
    act(() =>
      emitMockEvent('task://progress', {
        taskId: 'task-1',
        percent: 40,
        note: 'Reading abstracts',
        at: Date.now(),
      })
    )
    const bar = await screen.findByRole('progressbar')
    expect(bar).toHaveAttribute('aria-valuenow', '40')
    expect(screen.getByText('Reading abstracts')).toBeInTheDocument()
  })

  it('drops the bar, not to NaN, when progress has no percent', async () => {
    render(<TaskStatusPanel taskId="task-1" />)
    await screen.findByTestId('task-status-panel')
    act(() =>
      emitMockEvent('task://progress', {
        taskId: 'task-1',
        percent: 0,
        note: 'started',
        at: Date.now(),
      })
    )
    const bar = await screen.findByRole('progressbar')
    expect(bar).toHaveAttribute('aria-valuenow', '0')
    act(() =>
      emitMockEvent('task://progress', {
        taskId: 'task-1',
        percent: null,
        note: 'still working',
        at: Date.now(),
      })
    )
    expect(await screen.findByText('still working')).toBeInTheDocument()
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument()
    expect(screen.getByTestId('task-status-panel')).not.toHaveTextContent('NaN')
  })

  it('restores progress from the task messages after a remount', async () => {
    Object.assign(commands, {
      getTaskMessages: ok([
        {
          id: 1,
          kind: 'progress',
          payload: { percent: 0, note: 'started' },
          at: 1,
        },
        { id: 2, kind: 'note', payload: 'x', at: 2 },
        {
          id: 3,
          kind: 'progress',
          payload: { percent: 60, note: 'Halfway' },
          at: 3,
        },
      ]),
    })
    render(<TaskStatusPanel taskId="task-1" />)
    const bar = await screen.findByRole('progressbar')
    expect(bar).toHaveAttribute('aria-valuenow', '60')
    expect(screen.getByText('Halfway')).toBeInTheDocument()
  })

  it('says when a question waits for the user and opens the thread', async () => {
    const user = userEvent.setup()
    useNavigationStore.setState({ current: { kind: 'home' }, history: [] })
    Object.assign(commands, {
      getTask: ok(taskFixture({ state: 'inputRequired' })),
      getPendingClarifications: ok([
        {
          taskId: 'task-1',
          agentId: 'writer',
          question: {
            id: 'r1-q1',
            question: 'Which years?',
            topic: 'dates',
            requiresHuman: false,
          },
          deadline: null,
          approvalId: 'approval-1',
        },
      ]),
    })
    render(<TaskStatusPanel taskId="task-1" />)
    const notice = await screen.findByTestId('task-needs-answer')
    expect(notice).toHaveTextContent('Questions waiting for your answer: 1')
    expect(screen.getByTestId('task-status-panel')).toHaveTextContent(
      'Needs input'
    )
    await user.click(within(notice).getByRole('button', { name: 'Answer' }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'delegation-thread',
      taskId: 'task-1',
    })
  })

  it('says a submitted task waits for an offline member', async () => {
    Object.assign(commands, {
      getTask: ok(taskFixture({ state: 'submitted', acknowledgedAt: null })),
      getNetworkSnapshot: ok(networkSnapshot(false)),
    })
    render(<TaskStatusPanel taskId="task-1" />)
    expect(await screen.findByTestId('task-queued-offline')).toHaveTextContent(
      'Peer Person is offline. The request is queued'
    )
  })

  it('shows no queued notice while the member is online', async () => {
    Object.assign(commands, {
      getTask: ok(taskFixture({ state: 'submitted', acknowledgedAt: null })),
      getNetworkSnapshot: ok(networkSnapshot(true)),
    })
    render(<TaskStatusPanel taskId="task-1" />)
    await screen.findByTestId('task-status-panel')
    await waitFor(() => expect(mocks().getNetworkSnapshot).toHaveBeenCalled())
    expect(screen.queryByTestId('task-queued-offline')).not.toBeInTheDocument()
  })

  it('warns past twice the ETA', async () => {
    Object.assign(commands, {
      getTask: ok(
        taskFixture({ etaSeconds: 5, acknowledgedAt: Date.now() - 20_000 })
      ),
    })
    render(<TaskStatusPanel taskId="task-1" />)
    expect(await screen.findByText(/over twice the ETA/)).toBeInTheDocument()
  })

  it('renders nothing for a terminal task', async () => {
    Object.assign(commands, {
      getTask: ok(taskFixture({ state: 'completed' })),
    })
    const { container } = render(<TaskStatusPanel taskId="task-1" />)
    await waitFor(() => expect(mocks().getTask).toHaveBeenCalled())
    expect(container).toBeEmptyDOMElement()
  })

  it('hides when task://state_changed reports a terminal state', async () => {
    render(<TaskStatusPanel taskId="task-1" />)
    await screen.findByTestId('task-status-panel')
    const done = taskFixture({ state: 'completed', completedAt: Date.now() })
    act(() =>
      emitMockEvent('task://state_changed', {
        task: done,
        event: {
          taskId: 'task-1',
          fromState: 'working',
          toState: 'completed',
          reason: null,
          at: Date.now(),
        },
      })
    )
    await waitFor(() =>
      expect(screen.queryByTestId('task-status-panel')).not.toBeInTheDocument()
    )
  })
})

describe('ProviderStatePanel', () => {
  beforeEach(() => {
    Object.assign(commands, {
      getTaskMessages: ok([
        {
          id: 1,
          kind: 'clarification_request',
          payload: { taskId: 'task-1', round: 1, questions: [] },
          at: 1,
        },
      ]),
      getPendingClarifications: ok([
        {
          taskId: 'task-1',
          agentId: 'writer',
          question: {
            id: 'q1',
            question: 'Which year?',
            topic: 'dates',
            requiresHuman: false,
          },
          deadline: null,
          approvalId: null,
        },
        {
          taskId: 'other',
          agentId: 'writer',
          question: {
            id: 'q2',
            question: '?',
            topic: '',
            requiresHuman: false,
          },
          deadline: null,
          approvalId: null,
        },
      ]),
    })
  })

  it('shows the requester, round, and pending questions', async () => {
    render(<ProviderStatePanel task={taskFixture()} />)
    const panel = await screen.findByTestId('provider-state-panel')
    expect(panel).toHaveTextContent('Handling task from writer: find-papers')
    await waitFor(() => expect(panel).toHaveTextContent('round 1/2'))
    expect(await screen.findByText('Questions waiting: 1')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Send progress' })).toBeDisabled()
  })

  it('renders nothing once the task ends', () => {
    const { container } = render(
      <ProviderStatePanel task={taskFixture({ state: 'failed' })} />
    )
    expect(container).toBeEmptyDOMElement()
  })

  it('hands the task to a teammate who serves the capability', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      listReassignCandidates: ok([
        { instanceId: 'bob-1', displayName: 'Bob', cardId: 'bob-1/papers' },
      ]),
      reassignTask: ok(taskFixture({ state: 'reassigned' })),
    })
    render(<ProviderStatePanel task={taskFixture()} />)
    await user.click(await screen.findByTestId('reassign-task'))
    await user.click(await screen.findByRole('menuitem', { name: 'Bob' }))
    await waitFor(() =>
      expect(commands.reassignTask).toHaveBeenCalledWith({
        taskId: 'task-1',
        newProviderInstance: 'bob-1',
        reason: null,
      })
    )
  })

  it('offers no reassignment when nobody else serves it', async () => {
    Object.assign(commands, { listReassignCandidates: ok([]) })
    render(<ProviderStatePanel task={taskFixture()} />)
    await screen.findByTestId('provider-state-panel')
    await waitFor(() =>
      expect(commands.listReassignCandidates).toHaveBeenCalledWith('task-1')
    )
    expect(screen.queryByTestId('reassign-task')).toBeNull()
  })
})

describe('AgentTaskStrip', () => {
  it('shows a requester panel and a provider panel for in-flight tasks only', async () => {
    Object.assign(commands, {
      listTasks: ok([
        taskFixture({ id: 'sent' }),
        taskFixture({
          id: 'served',
          requesterAgent: 'peer/boss',
          requesterInstance: 'peer',
          providerAgent: 'writer',
          providerInstance: 'me',
        }),
        taskFixture({ id: 'done', state: 'completed' }),
      ]),
      getTask: vi.fn(async (id: string) => ({
        status: 'ok',
        data: taskFixture({ id }),
      })),
      getTaskMessages: ok([]),
      getPendingClarifications: ok([]),
    })
    render(<AgentTaskStrip agentId="writer" />)
    expect(await screen.findByTestId('task-status-panel')).toBeInTheDocument()
    expect(screen.getByTestId('provider-state-panel')).toBeInTheDocument()
    expect(mocks().listTasks).toHaveBeenCalledWith({
      state: null,
      role: null,
      agentId: 'writer',
      limit: 20,
    })
  })
})
