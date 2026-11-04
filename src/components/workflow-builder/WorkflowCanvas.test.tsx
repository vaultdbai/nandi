import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@/test/test-utils'
import { emitMockEvent } from '@/test/mock-events'
import { commands, type AgentInfo } from '@/lib/tauri-bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { WorkflowCanvas } from './WorkflowCanvas'
import {
  graphNode,
  mockBuilderCommands,
  mockReactFlowDom,
  sampleGraph,
  sampleWorkflow,
} from './test-support'

const node = (container: HTMLElement, id: string) =>
  container.querySelector<HTMLElement>(`[data-id="${id}"] [data-status]`)

describe('WorkflowCanvas', () => {
  beforeAll(mockReactFlowDom)

  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
    mockBuilderCommands()
    vi.mocked(commands.getWorkflow).mockResolvedValue({
      status: 'ok',
      data: sampleWorkflow(),
    })
  })

  it('renders a node per step and an edge per dependency', async () => {
    const { container } = render(<WorkflowCanvas workflowId="daily-report" />)

    expect(await screen.findByText('research')).toBeInTheDocument()
    expect(screen.getByText('Research Agent')).toBeInTheDocument()
    expect(screen.getByText('publish')).toBeInTheDocument()
    expect(screen.getByText('Asks you first')).toBeInTheDocument()
    expect(node(container, 'research')).toHaveAttribute('data-status', 'done')
    expect(node(container, 'publish')).toHaveAttribute('data-status', 'pending')
    await waitFor(() =>
      expect(
        container.querySelector('[data-id="research->publish"]')
      ).toBeInTheDocument()
    )
    expect(container.querySelectorAll('.react-flow__edge')).toHaveLength(1)
  })

  it('shows the clicked step in the details panel', async () => {
    const user = userEvent.setup()
    render(<WorkflowCanvas workflowId="daily-report" />)
    const panel = screen.getByRole('region', { name: 'Step details' })
    expect(
      within(panel).getByText('Select a step in the graph to see its details.')
    ).toBeInTheDocument()

    // A plain click: React Flow's pan/zoom reads `event.view` on mousedown,
    // which user-event leaves empty
    fireEvent.click(await screen.findByText('research'))

    // Named in words, never by its ID
    expect(
      within(panel).getByRole('heading', { name: 'Step: Research' })
    ).toBeInTheDocument()
    expect(within(panel).getByText('Success')).toBeInTheDocument()
    expect(within(panel).getByText('2s')).toBeInTheDocument()
    expect(within(panel).getByText('Three')).toBeInTheDocument()
    // Editable whatever the last run did
    expect(within(panel).getByText('Edit step')).toBeInTheDocument()

    await user.click(within(panel).getByRole('button', { name: 'View run' }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'run-detail',
      runId: 'run-a',
    })
  })

  it('edits a step through the YAML, picking its agent by name', async () => {
    const user = userEvent.setup()
    const { saveWorkflowYaml } = mockBuilderCommands()
    saveWorkflowYaml.mockResolvedValue({ status: 'ok', data: {} })
    vi.mocked(commands.listAgents).mockResolvedValue({
      status: 'ok',
      data: [
        { id: 'publisher-agent', name: 'Publisher' } as AgentInfo,
        { id: 'mailer', name: 'Mailer' } as AgentInfo,
      ],
    })
    render(<WorkflowCanvas workflowId="daily-report" />)

    fireEvent.click(await screen.findByText('publish'))
    const panel = screen.getByRole('region', { name: 'Step details' })
    const agent = await within(panel).findByLabelText('Agent')
    await within(panel).findByRole('option', { name: 'Mailer' })
    await user.selectOptions(agent, 'mailer')
    await user.click(within(panel).getByRole('button', { name: 'Save step' }))

    expect(saveWorkflowYaml).toHaveBeenCalledTimes(1)
    const [workflowId, yaml] = saveWorkflowYaml.mock.calls[0] as [
      string,
      string,
    ]
    expect(workflowId).toBe('daily-report')
    expect(yaml).toContain('  - id: "publish"\n    agent: "mailer"\n')
    expect(yaml).toContain('    requires_approval: true')
  })

  it('updates node status live from workflow events', async () => {
    const { getWorkflowGraph } = mockBuilderCommands()
    const { container } = render(<WorkflowCanvas workflowId="daily-report" />)
    await screen.findByText('publish')
    // What the refetch after the event returns
    const [research] = sampleGraph().nodes
    getWorkflowGraph.mockResolvedValue({
      status: 'ok',
      data: sampleGraph({
        runStatus: 'running',
        nodes: [
          research ?? graphNode(),
          graphNode({ id: 'publish', name: 'publish', status: 'running' }),
        ],
      }),
    })

    act(() => {
      emitMockEvent('workflow://step_started', {
        workflowId: 'daily-report',
        runId: 'wr-1',
        stepId: 'publish',
        agentId: 'publisher-agent',
        agentRunId: 'run-b',
        input: 'Send: Three stories',
      })
    })

    await waitFor(() =>
      expect(node(container, 'publish')).toHaveAttribute(
        'data-status',
        'running'
      )
    )
    // The edge into the running step is animated
    expect(
      container.querySelector('[data-id="research->publish"]')
    ).toHaveClass('animated')
  })
})
