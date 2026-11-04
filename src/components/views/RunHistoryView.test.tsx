import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, within } from '@/test/test-utils'
import type { RunDetail, RunSummary } from '@/lib/tauri-bindings'
import { run } from '@/test/fixtures'
import { useNavigationStore } from '@/store/navigation-store'
import { RunHistoryView } from './RunHistoryView'

const { mockClient } = vi.hoisted(() => ({
  mockClient: {
    getRunDetail: vi.fn(),
    listAllRuns: vi.fn(),
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

function serve(overrides: Partial<RunSummary>) {
  const detail: RunDetail = {
    run: run('run-1', 'writer', {
      status: 'completed',
      endedAt: run('x', 'writer').startedAt + 4_000,
      ...overrides,
    }),
    agentName: 'Writer',
    messages: [],
    events: [],
  }
  mockClient.getRunDetail.mockResolvedValue({ status: 'ok', data: detail })
}

/** The "Run details" side panel's value for one label */
async function metaValue(label: string) {
  const aside = await screen.findByRole('complementary', {
    name: 'Run details',
  })
  const term = within(aside).getByText(label)
  return term.nextElementSibling as HTMLElement
}

describe('RunHistoryView metadata', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockClient.listAllRuns.mockResolvedValue({
      status: 'ok',
      data: { rows: [], total: 0 },
    })
  })

  it('shows the model the run called as its engine', async () => {
    serve({ model: 'deepseek:deepseek-flash', inputTokens: 120 })
    render(<RunHistoryView runId="run-1" />)
    const engine = await metaValue('Engine')
    // The model's name in words; its id on hover
    expect(engine).toHaveTextContent('DeepSeek Flash')
    expect(within(engine).getByTitle('deepseek:deepseek-flash')).toBeTruthy()
    expect(engine).not.toHaveTextContent('built-in replies')
  })

  it('names built-in replies only for a run that finished without a model', async () => {
    serve({ model: null })
    render(<RunHistoryView runId="run-1" />)
    const engine = await metaValue('Engine')
    expect(engine).toHaveTextContent(
      'No model connected — this run used built-in replies'
    )
    expect(
      within(engine).getByRole('button', { name: 'Add a model' })
    ).toBeInTheDocument()
  })

  it('doesn’t call a failed run with no model call canned', async () => {
    serve({ model: null, status: 'failed' })
    render(<RunHistoryView runId="run-1" />)
    expect(await metaValue('Engine')).toHaveTextContent("Didn't reach a model")
  })

  it('shows who asked: the user, or the delegating agent', async () => {
    serve({})
    const { unmount } = render(<RunHistoryView runId="run-1" />)
    expect(await metaValue('Source')).toHaveTextContent('You')
    unmount()

    serve({
      sourceType: 'agent',
      sourceId: 'them/papers',
      sourceLabel: 'Delegated by Papers · summarize',
    })
    render(<RunHistoryView runId="run-2" />)
    expect(await metaValue('Source')).toHaveTextContent(
      'Delegated by Papers · summarize'
    )
  })

  it('falls back to the source type without a label', async () => {
    serve({ sourceType: 'webhook' })
    render(<RunHistoryView runId="run-1" />)
    expect(await metaValue('Source')).toHaveTextContent('Webhook')
  })
})

describe('RunHistoryView compaction lineage', () => {
  const root = run('run-a', 'writer', { status: 'compacted' })
  const child = run('run-b', 'writer', {
    status: 'compacted',
    parentRunId: 'run-a',
    compactionSummary: 'Objective: first',
    startedAt: root.startedAt + 1_000,
  })
  const grandchild = run('run-c', 'writer', {
    status: 'running',
    parentRunId: 'run-b',
    compactionSummary: 'Objective: second',
    startedAt: root.startedAt + 2_000,
  })
  const unrelated = run('run-z', 'writer', { status: 'completed' })

  beforeEach(() => {
    vi.clearAllMocks()
    mockClient.listAllRuns.mockResolvedValue({
      status: 'ok',
      data: { rows: [grandchild, unrelated, child, root], total: 4 },
    })
  })

  it('nandis each child run under the run it continues', async () => {
    serve({ ...child, endedAt: null })
    render(<RunHistoryView runId="run-b" />)

    const lineage = await screen.findByRole('navigation', {
      name: 'Compaction lineage',
    })
    expect(mockClient.listAllRuns).toHaveBeenCalledWith(
      { agentId: 'writer' },
      200,
      0
    )
    // run-a > run-b > run-c, and the unrelated run isn't part of it
    const [top] = within(lineage).getAllByRole('listitem')
    if (!top) throw new Error('no lineage')
    expect(within(top).getByText('run-a')).toBeInTheDocument()
    const nested = within(top).getAllByRole('listitem')
    expect(nested.map(item => item.querySelector('code')?.textContent)).toEqual(
      ['run-b', 'run-c']
    )
    const [first] = nested
    if (!first) throw new Error('no child')
    expect(within(first).getByText('run-c')).toBeInTheDocument()
    expect(within(lineage).queryByText('run-z')).not.toBeInTheDocument()
    expect(
      within(lineage).getByRole('button', { current: 'page' })
    ).toHaveTextContent('run-b')

    const user = userEvent.setup()
    await user.click(within(lineage).getByText('run-c'))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'run-detail',
      runId: 'run-c',
    })
  })

  it('shows no lineage for a run that was never compacted', async () => {
    serve({ id: 'run-z' })
    render(<RunHistoryView runId="run-z" />)
    await screen.findByRole('complementary', { name: 'Run details' })
    expect(
      screen.queryByRole('navigation', { name: 'Compaction lineage' })
    ).not.toBeInTheDocument()
  })
})
