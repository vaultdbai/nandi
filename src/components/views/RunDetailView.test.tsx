import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import type { AnalysisResponse } from '@/lib/bindings'
import type { ChatMessage, RunDetail } from '@/lib/tauri-bindings'
import {
  agentMessage,
  approvalMessage,
  run,
  userMessage,
} from '@/test/fixtures'
import { useNavigationStore } from '@/store/navigation-store'
import { RunDetailView } from './RunDetailView'

const { mockClient } = vi.hoisted(() => ({
  mockClient: {
    getRunDetail: vi.fn(),
    summarizeRun: vi.fn(),
    analyzeRun: vi.fn(),
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

const actions = vi.hoisted(() => ({
  cancelTurn: vi.fn(() => Promise.resolve()),
  cancelQueuedRun: vi.fn(() => Promise.resolve()),
  sendAgentMessage: vi.fn(() => Promise.resolve()),
  decideApproval: vi.fn(() => Promise.resolve(true)),
}))
vi.mock('@/lib/agent-actions', () => actions)

vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

const writeCall: ChatMessage = {
  kind: 'toolCall',
  id: 't1',
  runId: 'run-1',
  createdAt: run('x', 'w').startedAt + 1_000,
  tool: 'write_file',
  args: { path: 'out/report.md', content: 'hello' },
  status: 'done',
  result: 'ok',
  isError: false,
}

const OUTPUT = [
  'Quarterly numbers:',
  '',
  '| Quarter | Revenue |',
  '| --- | --- |',
  '| Q1 | 120 |',
  '| Q2 | 150 |',
].join('\n')

function fixture(overrides: Partial<RunDetail> = {}): RunDetail {
  return {
    run: run('run-1', 'writer', {
      status: 'completed',
      endedAt: run('x', 'writer').startedAt + 4_000,
      inputTokens: 1200,
      outputTokens: 300,
      costUsd: 0.0042,
      model: 'claude:claude-sonnet-5',
    }),
    agentName: 'Writer',
    messages: [
      userMessage('u1', 'Write the report', 'run-1'),
      writeCall,
      approvalMessage('a1', 'approved', 'run-1'),
      agentMessage('m1', OUTPUT, 'run-1'),
    ],
    events: [
      {
        id: 41,
        runId: 'run-1',
        kind: 'agent://run',
        payload: { run: { status: 'running' } },
        createdAt: run('x', 'w').startedAt,
      },
      {
        id: 42,
        runId: 'run-1',
        kind: 'agent://tool_result',
        payload: { result: 'permission denied', isError: true },
        createdAt: run('x', 'w').startedAt + 1_500,
      },
    ],
    ...overrides,
  }
}

const answer: AnalysisResponse = {
  summary: 'The agent wrote the report [artifact:out/report.md].',
  keyRisks: ['Nobody reviewed it'],
  sources: [
    {
      kind: 'artifact',
      id: 'out/report.md',
      label: 'out/report.md (written by write_file)',
      url: null,
    },
    { kind: 'event', id: '41', label: 'Event #41 · agent://run', url: null },
  ],
  question: 'What did it write?',
  createdAt: 0,
  cached: false,
}

describe('RunDetailView', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockClient.getRunDetail.mockResolvedValue(ok(fixture()))
    mockClient.summarizeRun.mockResolvedValue(
      ok('It wrote a report.\n\n## Key risks\n- None found')
    )
    mockClient.analyzeRun.mockResolvedValue(ok(answer))
  })

  it('shows the header, key results, and a chart from the output', async () => {
    render(<RunDetailView runId="run-1" />)
    // "agent · input · time", never the run ID
    const title = await screen.findByRole('heading', {
      name: /^Writer · Do the thing · \d/,
    })
    expect(title.textContent).not.toContain('run-1')
    expect(screen.getAllByText('Completed').length).toBeGreaterThan(0)
    expect(
      screen.getAllByText('claude:claude-sonnet-5', { selector: 'code' }).length
    ).toBeGreaterThan(0)
    expect(screen.getByText('1.5K')).toBeInTheDocument()
    // The table's numbers become cards and a chart
    const numbers = screen.getByRole('region', {
      name: 'Numbers from the output',
    })
    expect(within(numbers).getByText('120')).toBeInTheDocument()
    expect(
      within(numbers).getByRole('img', { name: 'Revenue' })
    ).toBeInTheDocument()
    expect(screen.getByText('Generated files (1)')).toBeInTheDocument()
  })

  it('renders every tab', async () => {
    const user = userEvent.setup()
    render(<RunDetailView runId="run-1" />)
    await screen.findByRole('heading', { name: /^Writer · Do the thing/ })

    await user.click(screen.getByRole('tab', { name: 'Timeline' }))
    const timeline = screen.getByRole('list', { name: 'Timeline' })
    expect(within(timeline).getByText('Run started')).toBeInTheDocument()
    expect(within(timeline).getByText('Save a file')).toBeInTheDocument()
    expect(within(timeline).getByText('Run Completed')).toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: 'Outputs' }))
    expect(screen.getByText('Final output')).toBeInTheDocument()
    expect(screen.getByText('Quarterly numbers:')).toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: 'Files' }))
    const table = screen.getByRole('table')
    expect(within(table).getByText('out/report.md')).toBeInTheDocument()
    expect(within(table).getByText('5 B')).toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: 'Logs' }))
    // Readable lines; the raw kind and JSON only behind the row
    expect(screen.getByText('Run is now running')).toBeInTheDocument()
    expect(screen.queryByText(/agent:\/\/run/)).not.toBeInTheDocument()
    await user.click(screen.getByText('Run is now running'))
    expect(screen.getByText(/#41 · agent:\/\/run/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Copy log' })).toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: 'Summary' }))
    expect(
      await within(screen.getByRole('tabpanel')).findByText(
        'It wrote a report.'
      )
    ).toBeInTheDocument()

    await user.click(screen.getByRole('tab', { name: /Approvals/ }))
    expect(screen.getByText('Approved')).toBeInTheDocument()
  })

  it("approves a pending call from the run's conversation", async () => {
    const user = userEvent.setup()
    mockClient.getRunDetail.mockResolvedValue(
      ok(
        fixture({
          messages: [
            userMessage('u1', 'Write the report', 'run-1'),
            approvalMessage('a2', 'pending', 'run-1'),
          ],
        })
      )
    )
    render(<RunDetailView runId="run-1" />)
    await screen.findByRole('heading', { name: /^Writer · Do the thing/ })
    await user.click(screen.getByRole('tab', { name: 'Outputs' }))
    await user.click(screen.getByText(/Full conversation/))
    const card = await screen.findByTestId('approval-card')
    await user.click(within(card).getByRole('button', { name: /Approve/ }))
    expect(actions.decideApproval).toHaveBeenCalledWith(
      'writer',
      'a2',
      'approved'
    )
  })

  it('summarizes only on the Summary tab', async () => {
    const user = userEvent.setup()
    render(<RunDetailView runId="run-1" />)
    await screen.findByRole('heading', { name: /^Writer · Do the thing/ })
    expect(mockClient.summarizeRun).not.toHaveBeenCalled()

    await user.click(screen.getByRole('tab', { name: 'Summary' }))
    expect(await screen.findByText('It wrote a report.')).toBeInTheDocument()
    expect(mockClient.summarizeRun).toHaveBeenCalledWith('run-1')
  })

  it('asks from any tab, opens Summary, and answers with sources', async () => {
    const user = userEvent.setup()
    render(<RunDetailView runId="run-1" />)
    await screen.findByRole('heading', { name: /^Writer · Do the thing/ })
    await user.click(screen.getByRole('tab', { name: 'Files' }))

    await user.type(
      screen.getByRole('textbox', { name: 'Ask a question about this run…' }),
      'What did it write?'
    )
    await user.click(screen.getByRole('button', { name: 'Send' }))

    expect(screen.getByRole('tab', { name: 'Summary' })).toHaveAttribute(
      'aria-selected',
      'true'
    )
    expect(await screen.findByText('It wrote a report.')).toBeInTheDocument()
    expect(mockClient.analyzeRun).toHaveBeenCalledWith(
      'run-1',
      'What did it write?'
    )
    expect(
      await screen.findByText(/The agent wrote the report/)
    ).toBeInTheDocument()
    expect(screen.getByText('Nobody reviewed it')).toBeInTheDocument()
    const sources = screen.getByRole('region', { name: 'Sources' })
    expect(
      within(sources).getByText('Event #41 · agent://run')
    ).toBeInTheDocument()

    // A cited event opens the log, highlighted
    await user.click(within(sources).getByText('Event #41 · agent://run'))
    expect(screen.getByRole('tab', { name: 'Logs' })).toHaveAttribute(
      'aria-selected',
      'true'
    )
  })

  it('explains how to add a key when none is configured', async () => {
    mockClient.summarizeRun.mockResolvedValue({
      status: 'error',
      error: 'Add an API key in Settings to use run analysis',
    })
    render(<RunDetailView runId="run-1" tab="summary" />)
    expect(
      await screen.findByRole('button', { name: 'Open Settings' })
    ).toBeInTheDocument()
  })

  it("copies the run ID and links to the agent's runs from the actions menu", async () => {
    const user = userEvent.setup()
    const writeText = vi.fn(() => Promise.resolve())
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    })
    render(<RunDetailView runId="run-1" />)
    await screen.findByRole('heading', { name: /^Writer · Do the thing/ })

    await user.click(screen.getByRole('button', { name: 'Actions' }))
    await user.click(screen.getByRole('menuitem', { name: 'Copy run ID' }))
    await waitFor(() => expect(writeText).toHaveBeenCalledWith('run-1'))

    await user.click(screen.getByRole('button', { name: 'Actions' }))
    await user.click(
      screen.getByRole('menuitem', { name: "View this agent's runs" })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'agent',
      agentId: 'writer',
      tab: 'runs',
    })
  })

  it('filters the log by kind', async () => {
    const user = userEvent.setup()
    render(<RunDetailView runId="run-1" tab="logs" />)
    expect(
      await screen.findByText('Tool failed: permission denied')
    ).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Error' }))
    expect(screen.queryByText('Run is now running')).not.toBeInTheDocument()
    expect(
      screen.getByText('Tool failed: permission denied')
    ).toBeInTheDocument()

    await user.type(screen.getByLabelText('Search the log'), 'nothing like it')
    expect(screen.getByText('No events match.')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Clear filters' }))
    expect(screen.getByText('Run is now running')).toBeInTheDocument()
  })

  it('re-runs a run that wrote files only after asking, and stays put', async () => {
    const user = userEvent.setup()
    useNavigationStore.setState({
      current: { kind: 'run-detail', runId: 'run-1' },
      history: [],
    })
    render(<RunDetailView runId="run-1" />)
    // Ended: Re-run, no Stop
    expect(screen.queryByRole('button', { name: 'Stop' })).toBeNull()
    await user.click(await screen.findByRole('button', { name: 'Re-run' }))
    expect(actions.sendAgentMessage).not.toHaveBeenCalled()
    expect(
      screen.getByText('Send the same input to Writer again?')
    ).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Re-run' }))

    await waitFor(() =>
      expect(actions.sendAgentMessage).toHaveBeenCalledWith(
        'writer',
        'Do the thing'
      )
    )
    const { toast } = await import('sonner')
    await waitFor(() =>
      expect(toast.success).toHaveBeenCalledWith(
        'Sent the same input to Writer',
        expect.anything()
      )
    )
    expect(useNavigationStore.getState().current.kind).toBe('run-detail')
  })

  it('re-runs a read-only run at once', async () => {
    const user = userEvent.setup()
    mockClient.getRunDetail.mockResolvedValue(
      ok(fixture({ messages: [agentMessage('m1', 'Nothing to do', 'run-1')] }))
    )
    render(<RunDetailView runId="run-1" />)
    await user.click(await screen.findByRole('button', { name: 'Re-run' }))
    await waitFor(() => expect(actions.sendAgentMessage).toHaveBeenCalled())
    expect(screen.queryByRole('alertdialog')).toBeNull()
  })

  it('names a run without a model as a setup problem', async () => {
    mockClient.getRunDetail.mockResolvedValue(
      ok(fixture({ run: run('run-1', 'writer', { status: 'completed' }) }))
    )
    render(<RunDetailView runId="run-1" />)
    expect(
      await screen.findByText(
        'No model connected — this run used built-in replies'
      )
    ).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Add a model' })
    ).toBeInTheDocument()
    expect(screen.queryByText(/Canned responses/)).toBeNull()
  })

  it('shows a text summary when the output has no numbers', async () => {
    mockClient.getRunDetail.mockResolvedValue(
      ok(
        fixture({
          messages: [
            agentMessage('m1', 'All good, nothing to report.', 'run-1'),
          ],
        })
      )
    )
    render(<RunDetailView runId="run-1" />)
    expect(
      await screen.findByText('All good, nothing to report.', {
        selector: 'p',
      })
    ).toBeInTheDocument()
    expect(
      screen.queryByRole('region', { name: 'Numbers from the output' })
    ).not.toBeInTheDocument()
  })

  it('says when the run is missing', async () => {
    mockClient.getRunDetail.mockResolvedValue({
      status: 'error',
      error: 'Run nope not found',
    })
    render(<RunDetailView runId="nope" />)
    expect(await screen.findByText('Run nope not found')).toBeInTheDocument()
    expect(screen.getByText("Couldn't load this run.")).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Try again' })
    ).toBeInTheDocument()
  })
})

describe('RunDetailView while running', () => {
  const going = (overrides: Partial<RunDetail> = {}) =>
    fixture({
      run: run('run-1', 'writer', { status: 'running' }),
      ...overrides,
    })

  beforeEach(() => {
    vi.clearAllMocks()
    mockClient.summarizeRun.mockResolvedValue(ok('Summary.'))
  })

  it('opens on the timeline with Stop instead of Re-run', async () => {
    mockClient.getRunDetail.mockResolvedValue(ok(going()))
    render(<RunDetailView runId="run-1" />)
    expect(
      await screen.findByRole('button', { name: 'Stop' })
    ).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Re-run' })).toBeNull()
    expect(screen.getByRole('tab', { name: 'Timeline' })).toHaveAttribute(
      'aria-selected',
      'true'
    )
  })

  it('asks before stopping a run that wrote files', async () => {
    const user = userEvent.setup()
    mockClient.getRunDetail.mockResolvedValue(ok(going()))
    render(<RunDetailView runId="run-1" />)
    await user.click(await screen.findByRole('button', { name: 'Stop' }))
    expect(actions.cancelTurn).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: 'Stop run' }))
    await waitFor(() =>
      expect(actions.cancelTurn).toHaveBeenCalledWith('writer', 'run-1')
    )
  })

  it('stops a read-only run at once, and a queued one from the queue', async () => {
    const user = userEvent.setup()
    mockClient.getRunDetail.mockResolvedValue(
      ok(going({ messages: [userMessage('u1', 'Hi', 'run-1')] }))
    )
    const { unmount } = render(<RunDetailView runId="run-1" />)
    await user.click(await screen.findByRole('button', { name: 'Stop' }))
    await waitFor(() => expect(actions.cancelTurn).toHaveBeenCalled())
    unmount()

    mockClient.getRunDetail.mockResolvedValue(
      ok(fixture({ run: run('run-1', 'writer', { status: 'queued' }) }))
    )
    render(<RunDetailView runId="run-1" />)
    await user.click(await screen.findByRole('button', { name: 'Stop' }))
    await waitFor(() =>
      expect(actions.cancelQueuedRun).toHaveBeenCalledWith('run-1')
    )
  })

  it('says a CLI runtime decides its own tool use', async () => {
    const user = userEvent.setup()
    mockClient.getRunDetail.mockResolvedValue(
      ok(
        going({
          messages: [{ ...writeCall, id: 'harness-s1-t1', tool: 'Edit a.ts' }],
          events: [
            {
              id: 50,
              runId: 'run-1',
              kind: 'harness://permission_request',
              payload: { title: 'Edit a.ts', decision: 'allow_once' },
              createdAt: run('x', 'w').startedAt + 500,
            },
          ],
        })
      )
    )
    render(<RunDetailView runId="run-1" tab="approvals" />)
    expect(
      await screen.findByText(/Nothing here needed your approval/)
    ).toBeInTheDocument()
    expect(screen.getByText('Allowed automatically')).toBeInTheDocument()
    expect(screen.queryByText(/Needs approval/)).toBeNull()
    await user.click(
      screen.getByRole('button', { name: 'Change in agent settings' })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'agent',
      agentId: 'writer',
      tab: 'settings',
    })
  })
})

describe('RunDetailView compaction', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockClient.summarizeRun.mockResolvedValue(ok('Summary.'))
  })

  it('links to the run it continues and shows the compacted context', async () => {
    const user = userEvent.setup()
    const parentRunId = '0f9e8d7c-1111-2222-3333-444455556666'
    mockClient.getRunDetail.mockResolvedValue(
      ok(
        fixture({
          run: run('run-2', 'writer', {
            status: 'running',
            parentRunId,
            compactionSummary:
              'Objective: ship the report\n\nNext Move: run the checks',
          }),
        })
      )
    )
    // Running opens on the timeline; the compacted context is on Overview
    render(<RunDetailView runId="run-2" tab="overview" />)

    const link = await screen.findByRole('button', {
      name: 'Continued from run 0f9e8d7c',
    })
    // Collapsed until opened
    const trigger = screen.getByRole('button', { name: 'Compacted context' })
    expect(trigger).toHaveAttribute('aria-expanded', 'false')
    await user.click(trigger)
    expect(trigger).toHaveAttribute('aria-expanded', 'true')
    expect(screen.getByText('Objective: ship the report')).toBeInTheDocument()
    expect(screen.getByText('Next Move: run the checks')).toBeInTheDocument()

    await user.click(link)
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'run-detail',
      runId: parentRunId,
    })
  })

  it('shows the compacted status and no lineage for a plain run', async () => {
    mockClient.getRunDetail.mockResolvedValue(
      ok(fixture({ run: run('run-1', 'writer', { status: 'compacted' }) }))
    )
    render(<RunDetailView runId="run-1" />)
    expect(await screen.findAllByText('Compacted')).not.toHaveLength(0)
    expect(
      screen.queryByRole('button', { name: /Continued from run/ })
    ).not.toBeInTheDocument()
    expect(
      screen.queryByRole('button', { name: 'Compacted context' })
    ).not.toBeInTheDocument()
  })
})
