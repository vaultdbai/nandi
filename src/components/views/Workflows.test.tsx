import { describe, it, expect, beforeAll, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type {
  AgentInfo,
  Trigger,
  Workflow,
  WorkflowRun,
} from '@/lib/tauri-bindings'
import { useNavigationStore } from '@/store/navigation-store'
import {
  mockBuilderCommands,
  mockReactFlowDom,
  sampleGraph,
} from '@/components/workflow-builder/test-support'
import { WorkflowDetailView } from './WorkflowDetailView'
import { WorkflowsListView } from './WorkflowsListView'

const T0 = Date.UTC(2026, 9, 1, 12)

function workflowRun(overrides: Partial<WorkflowRun> = {}): WorkflowRun {
  return {
    id: 'wr-1',
    workflowId: 'daily-report',
    status: 'completed',
    startedAt: T0,
    endedAt: T0 + 5_000,
    error: null,
    source: { kind: 'manual' },
    steps: [
      {
        stepId: 'research',
        agentId: 'research-agent',
        template: "Find today's AI news",
        requiresApproval: false,
        approved: false,
        status: 'done',
        input: "Find today's AI news",
        output: 'Three stories',
        agentRunId: 'run-a',
        error: null,
        startedAt: T0,
        endedAt: T0 + 2_000,
      },
      {
        stepId: 'publish',
        agentId: 'publisher-agent',
        template: 'Send: {{research.output}}',
        requiresApproval: true,
        approved: true,
        status: 'done',
        input: 'Send: Three stories',
        output: 'Sent',
        agentRunId: 'run-b',
        error: null,
        startedAt: T0 + 2_000,
        endedAt: T0 + 5_000,
      },
    ],
    ...overrides,
  }
}

function workflow(runs: WorkflowRun[] = [workflowRun()]): Workflow {
  return {
    id: 'daily-report',
    name: 'Daily Report',
    path: '/repo/workflows/daily-report.yaml',
    description: null,
    source: 'id: daily-report\n',
    steps: [
      {
        id: 'research',
        agent: 'research-agent',
        input: "Find today's AI news",
        requiresApproval: false,
      },
      {
        id: 'publish',
        agent: 'publisher-agent',
        input: 'Send: {{research.output}}',
        requiresApproval: true,
      },
    ],
    runs,
  }
}

function cronTrigger(): Trigger {
  return {
    id: 'morning',
    name: 'Morning news',
    enabled: true,
    sourceType: 'cron',
    sourceConfig: { schedule: '0 9 * * 1-5' },
    targetType: 'workflow',
    targetId: 'daily-report',
    inputTemplate: '',
    priority: 80,
    rateLimitSeconds: null,
    createdAt: T0,
    updatedAt: T0,
    lastFiredAt: null,
    hasSecret: false,
  }
}

describe('Workflows views', () => {
  beforeAll(mockReactFlowDom)

  beforeEach(() => {
    mockBuilderCommands()
    useNavigationStore.setState(useNavigationStore.getInitialState())
    vi.mocked(commands.getWorkflows).mockResolvedValue({
      status: 'ok',
      data: [
        {
          id: 'daily-report',
          name: 'Daily Report',
          definition: { steps: [{}, {}] },
          createdAt: T0,
        },
      ],
    })
    vi.mocked(commands.getWorkflow).mockResolvedValue({
      status: 'ok',
      data: workflow(),
    })
    vi.mocked(commands.runWorkflow).mockClear()
    vi.mocked(commands.approveWorkflowStep).mockClear()
    vi.mocked(commands.listAgents).mockResolvedValue({ status: 'ok', data: [] })
    vi.mocked(commands.listTriggers).mockResolvedValue({
      status: 'ok',
      data: [],
    })
  })

  it('lists workflows with what starts them and their last run, and runs one', async () => {
    const user = userEvent.setup()
    vi.mocked(commands.listTriggers).mockResolvedValue({
      status: 'ok',
      data: [cronTrigger()],
    })
    render(<WorkflowsListView />)

    const row = (await screen.findByText('Daily Report')).closest(
      'tr'
    ) as HTMLElement
    expect(within(row).getByText('2')).toBeInTheDocument()
    expect(
      within(row).getByText('At 09:00, Monday through Friday')
    ).toBeInTheDocument()
    expect(await within(row).findByText('Completed')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Run Daily Report' }))
    expect(commands.runWorkflow).toHaveBeenCalledWith('daily-report')

    await user.click(screen.getByRole('button', { name: 'Daily Report' }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'workflow',
      workflowId: 'daily-report',
    })
  })

  it('starts an empty list from a template, choosing agents by name', async () => {
    const user = userEvent.setup()
    vi.mocked(commands.getWorkflows).mockResolvedValue({
      status: 'ok',
      data: [],
    })
    vi.mocked(commands.listAgents).mockResolvedValue({
      status: 'ok',
      data: [{ id: 'writer', name: 'Writer' } as AgentInfo],
    })
    vi.mocked(commands.listWorkflowTemplates).mockResolvedValue({
      status: 'ok',
      data: [
        {
          id: 'daily-summary',
          name: 'Daily summary',
          steps: [
            {
              id: 'summarize',
              needsAgent: true,
              requiresApproval: false,
              manual: false,
            },
            {
              id: 'follow-up',
              needsAgent: false,
              requiresApproval: false,
              manual: true,
            },
          ],
        },
      ],
    })
    vi.mocked(commands.createWorkflowFromTemplate).mockResolvedValue({
      status: 'ok',
      data: { ...workflow([]), id: 'daily-summary', name: 'Daily summary' },
    })
    render(<WorkflowsListView />)

    expect(await screen.findByText('No workflows yet')).toBeInTheDocument()
    await user.click(
      screen.getByRole('button', { name: 'Start from a template' })
    )
    const dialog = await screen.findByRole('dialog')
    expect(
      await within(dialog).findByText(
        'An agent summarizes the day; you read it and follow up.'
      )
    ).toBeInTheDocument()
    const create = within(dialog).getByRole('button', {
      name: 'Create workflow',
    })
    // Every agent step needs its agent first
    expect(create).toBeDisabled()
    await user.selectOptions(
      within(dialog).getByLabelText('Agent for “Summarize”'),
      'writer'
    )
    await user.click(create)

    expect(commands.createWorkflowFromTemplate).toHaveBeenCalledWith({
      templateId: 'daily-summary',
      name: 'Daily summary',
      agents: { summarize: 'writer' },
    })
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'workflow',
      workflowId: 'daily-summary',
      tab: 'graph',
    })
  })

  it('opens on Overview: what it does, steps by agent, and recent runs', async () => {
    const user = userEvent.setup()
    vi.mocked(commands.listAgents).mockResolvedValue({
      status: 'ok',
      data: [
        { id: 'research-agent', name: 'Researcher' } as AgentInfo,
        { id: 'publisher-agent', name: 'Publisher' } as AgentInfo,
      ],
    })
    vi.mocked(commands.getWorkflow).mockResolvedValue({
      status: 'ok',
      data: { ...workflow(), description: 'Sends the news every morning.' },
    })
    render(<WorkflowDetailView workflowId="daily-report" />)

    expect(
      await screen.findByRole('heading', { name: 'Daily Report' })
    ).toBeInTheDocument()
    // No file path in the header
    expect(screen.queryByText(/daily-report\.yaml/)).not.toBeInTheDocument()
    expect(
      screen.getByText('Sends the news every morning.')
    ).toBeInTheDocument()
    expect(
      await screen.findByText(/Publisher · asks you first/)
    ).toBeInTheDocument()
    expect(
      screen.getByText(
        'Nothing starts it on its own. It runs when you click Run.'
      )
    ).toBeInTheDocument()

    // A recent run opens its request
    await user.click(screen.getByRole('button', { name: /Completed/ }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'work-item',
      id: 'wr-1',
    })
  })

  it('shows past runs with who started them and their transcript', async () => {
    const user = userEvent.setup()
    vi.mocked(commands.getWorkflow).mockResolvedValue({
      status: 'ok',
      data: workflow([
        workflowRun({ source: { kind: 'trigger', id: 'morning' } }),
      ]),
    })
    vi.mocked(commands.listTriggers).mockResolvedValue({
      status: 'ok',
      data: [cronTrigger()],
    })
    render(<WorkflowDetailView workflowId="daily-report" initialTab="runs" />)

    expect(await screen.findByText('Trigger: Morning news')).toBeInTheDocument()
    expect(screen.getByText('Send: Three stories')).toBeInTheDocument()
    expect(screen.getByText('Sent')).toBeInTheDocument()

    const [firstOpenRun] = screen.getAllByRole('button', { name: 'Open run' })
    if (!firstOpenRun) throw new Error('no Open run button')
    await user.click(firstOpenRun)
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'run',
      agentId: 'research-agent',
      runId: 'run-a',
    })
  })

  it('confirms before running a failed workflow again', async () => {
    const user = userEvent.setup()
    const failed = workflowRun({
      status: 'failed',
      error: 'Mail server down',
      steps: workflowRun().steps.map(step =>
        step.stepId === 'publish'
          ? {
              ...step,
              status: 'failed',
              output: null,
              error: 'Mail server down',
            }
          : step
      ),
    })
    vi.mocked(commands.getWorkflow).mockResolvedValue({
      status: 'ok',
      data: workflow([failed]),
    })
    render(<WorkflowDetailView workflowId="daily-report" />)

    expect(await screen.findByText('Stopped at “Publish”.')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Run again' }))
    const confirm = await screen.findByRole('alertdialog')
    expect(within(confirm).getByText(/Research, Publish/)).toBeInTheDocument()
    expect(commands.runWorkflow).not.toHaveBeenCalled()
    await user.click(within(confirm).getByRole('button', { name: 'Run again' }))
    expect(commands.runWorkflow).toHaveBeenCalledWith('daily-report')
  })

  it('approves a waiting step from the banner, by name', async () => {
    const user = userEvent.setup()
    const done = workflowRun()
    const waiting = workflowRun({
      status: 'awaitingApproval',
      endedAt: null,
      steps: done.steps.map(step =>
        step.stepId === 'publish'
          ? {
              ...step,
              status: 'awaitingApproval',
              approved: false,
              output: null,
              agentRunId: null,
              endedAt: null,
            }
          : step
      ),
    })
    vi.mocked(commands.getWorkflow).mockResolvedValue({
      status: 'ok',
      data: workflow([waiting]),
    })
    render(<WorkflowDetailView workflowId="daily-report" />)

    expect(
      await screen.findByText(/“Publish” \(publisher-agent\) needs you/)
    ).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Approve' }))
    expect(commands.approveWorkflowStep).toHaveBeenCalledWith(
      'wr-1',
      'publish',
      'approved'
    )
    // A run is in progress, so another can't start
    expect(screen.getByRole('button', { name: 'Run' })).toBeDisabled()
  })

  it('saves the workflow file, holding navigation while it has changes', async () => {
    const user = userEvent.setup()
    const { saveWorkflowYaml } = mockBuilderCommands(sampleGraph())
    saveWorkflowYaml
      .mockResolvedValueOnce({
        status: 'error',
        error: 'Invalid workflow YAML',
      })
      .mockResolvedValueOnce({ status: 'ok', data: workflow() })
    render(<WorkflowDetailView workflowId="daily-report" />)

    await user.click(await screen.findByRole('tab', { name: 'Configuration' }))
    const editor = screen.getByLabelText('Workflow file (advanced)')
    expect(editor).toHaveValue('id: daily-report\n')
    const save = screen.getByRole('button', { name: 'Save' })
    expect(save).toBeDisabled()

    await user.type(editor, 'name: Renamed')
    expect(useNavigationStore.getState().leaveGuarded).toBe(true)
    // The draft survives a tab switch
    await user.click(screen.getByRole('tab', { name: 'Overview' }))
    await user.click(screen.getByRole('tab', { name: /Configuration/ }))
    expect(screen.getByLabelText('Workflow file (advanced)')).toHaveValue(
      'id: daily-report\nname: Renamed'
    )

    await user.click(screen.getByRole('button', { name: 'Save' }))
    expect(saveWorkflowYaml).toHaveBeenCalledWith(
      'daily-report',
      'id: daily-report\nname: Renamed'
    )
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Invalid workflow YAML'
    )

    await user.click(screen.getByRole('button', { name: 'Save' }))
    expect(saveWorkflowYaml).toHaveBeenCalledTimes(2)
    await vi.waitFor(() =>
      expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    )
  })
})
