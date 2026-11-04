import { beforeEach, describe, expect, it, vi } from 'vitest'
import { streamAsksThroughAsk } from '@/test/ask-stream'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import type { WorkItem, WorkflowRun } from '@/lib/bindings'
import { comingUp, estimatedFinish, timeline } from '@/lib/work'
import { WorkflowProjectView } from './WorkflowProjectView'

const { mockClient, handlers } = vi.hoisted(() => {
  const handlers: ((event: unknown) => void)[] = []
  return {
    handlers,
    mockClient: {
      getWorkItem: vi.fn(),
      getWorkflow: vi.fn(),
      listAgents: vi.fn(),
      pauseWork: vi.fn(),
      resumeWork: vi.fn(),
      cancelWork: vi.fn(),
      pickAttachments: vi.fn(),
      ask: vi.fn(),
      supports: { attachments: false },
      subscribe: vi.fn((handler: (event: unknown) => void) => {
        handlers.push(handler)
        return { ready: Promise.resolve(), unsubscribe: vi.fn() }
      }),
    },
  }
})

vi.mock('@/lib/client', () => ({
  client: mockClient,
  isDesktop: () => false,
}))

// The graph needs a real layout engine; its presence is what matters here
vi.mock('@/components/workflow-builder', () => ({
  WorkflowCanvas: ({ workflowId }: { workflowId: string }) => (
    <div data-testid="workflow-canvas">{workflowId}</div>
  ),
}))

const ok = <T,>(data: T) => ({ status: 'ok' as const, data })
const NOW = Date.now()

function item(overrides: Partial<WorkItem>): WorkItem {
  return {
    id: 'run-1',
    kind: 'workflow',
    title: 'Onboarding Aayushi',
    state: 'active',
    owner: null,
    progress: { completedSteps: 6, totalSteps: 15, percent: 40 },
    nextAction: {
      description: 'writer is working on step 7',
      kind: 'auto',
      deadline: null,
      target: {
        runId: 'run-1',
        stepId: 'accounts',
        agentId: 'writer',
        taskId: null,
      },
    },
    workflowId: 'onboarding',
    rating: null,
    startedAt: NOW - 3_600_000,
    updatedAt: NOW - 60_000,
    ...overrides,
  }
}

const step = (
  stepId: string,
  status: WorkflowRun['steps'][number]['status'],
  startedAt: number | null,
  endedAt: number | null,
  extra: Partial<WorkflowRun['steps'][number]> = {}
): WorkflowRun['steps'][number] => ({
  stepId,
  agentId: 'writer',
  template: 'x',
  requiresApproval: false,
  status,
  input: null,
  output: status === 'done' ? `${stepId} output` : null,
  agentRunId: null,
  error: null,
  startedAt,
  endedAt,
  ...extra,
})

const run: WorkflowRun = {
  id: 'run-1',
  workflowId: 'onboarding',
  status: 'running',
  startedAt: NOW - 600_000,
  endedAt: null,
  error: null,
  source: null,
  steps: [
    step('welcome', 'done', NOW - 600_000, NOW - 540_000),
    step('accounts', 'running', NOW - 30_000, null),
    step('laptop', 'pending', null, null),
    step('review', 'pending', null, null, { requiresApproval: true }),
  ],
}

// The ask box streams; answers come from the `ask` mock
streamAsksThroughAsk(mockClient)

describe('WorkflowProjectView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    handlers.length = 0
    mockClient.getWorkItem.mockResolvedValue(ok(item({})))
    mockClient.listAgents.mockResolvedValue(
      ok([{ id: 'writer', name: 'Writer' }])
    )
    mockClient.getWorkflow.mockResolvedValue(
      ok({
        id: 'onboarding',
        name: 'Onboarding',
        steps: [],
        source: '',
        path: '',
        runs: [run],
      })
    )
  })

  it('shows the header, graph, timeline, and what comes next', async () => {
    render(<WorkflowProjectView workId="run-1" />)
    expect(await screen.findByText('Onboarding Aayushi')).toBeInTheDocument()
    expect(screen.getByText(/Owner: you · Started/)).toBeInTheDocument()
    expect(screen.getByRole('progressbar')).toHaveAttribute(
      'aria-valuenow',
      '40'
    )
    expect(screen.getByTestId('workflow-canvas')).toHaveTextContent(
      'onboarding'
    )

    const timelineRegion = await screen.findByRole('region', {
      name: 'Timeline',
    })
    // Agents by name, steps as words: no raw IDs on screen
    expect(
      await screen.findByText('Writer is working on Accounts')
    ).toBeInTheDocument()
    expect(timelineRegion).toHaveTextContent('Welcome done by Writer')
    expect(timelineRegion).toHaveTextContent('Accounts started on Writer')
    const next = screen.getByRole('region', { name: 'Coming up' })
    expect(next).toHaveTextContent('Laptop on Writer')
    expect(next).toHaveTextContent('Needs your approval')
  })

  it('pauses and cancels the run', async () => {
    mockClient.pauseWork.mockResolvedValue(ok(null))
    mockClient.cancelWork.mockResolvedValue(ok(null))
    const user = userEvent.setup()
    render(<WorkflowProjectView workId="run-1" />)
    await user.click(await screen.findByRole('button', { name: 'Pause' }))
    await waitFor(() =>
      expect(mockClient.pauseWork).toHaveBeenCalledWith('run-1')
    )
    await user.click(screen.getByRole('button', { name: 'Cancel' }))
    await waitFor(() =>
      expect(mockClient.cancelWork).toHaveBeenCalledWith('run-1')
    )
  })

  it('routes its ask box with the workflow as context', async () => {
    mockClient.ask.mockResolvedValue(
      ok({ kind: 'clarification', data: 'Which step?' })
    )
    const user = userEvent.setup()
    render(<WorkflowProjectView workId="run-1" />)
    await screen.findByRole('region', { name: 'Timeline' })
    await user.type(
      screen.getByRole('textbox'),
      'Add a step after laptop{Enter}'
    )
    await waitFor(() =>
      expect(mockClient.ask).toHaveBeenCalledWith({
        text: 'Add a step after laptop',
        attachments: [],
        context: {
          currentView: 'workflow:onboarding',
          focusedArtifact: { kind: 'workflow', id: 'onboarding' },
          recentRuns: ['run-1'],
        },
      })
    )
  })
})

describe('work helpers', () => {
  it('builds a timeline, what comes next, and an ETA', () => {
    const events = timeline(run).map(e => `${e.kind}:${e.stepId ?? ''}`)
    expect(events).toEqual([
      'runStarted:',
      'stepStarted:welcome',
      'stepDone:welcome',
      'stepStarted:accounts',
    ])
    const upcoming = comingUp(run, NOW)
    expect(upcoming.map(u => u.step.stepId)).toEqual(['laptop', 'review'])
    // One minute per step so far: accounts ends in ~30s, then a minute each
    expect(upcoming[0]?.expectedAt).toBe(NOW + 30_000)
    expect(upcoming[1]?.expectedAt).toBe(NOW + 90_000)
    expect(upcoming[1]?.needsApproval).toBe(true)
    expect(estimatedFinish(run, NOW)).toBe(NOW + 150_000)
    expect(estimatedFinish({ ...run, status: 'completed' }, NOW)).toBeNull()
  })
})
