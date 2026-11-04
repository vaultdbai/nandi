import { beforeEach, describe, expect, it, vi } from 'vitest'
import { streamAsksThroughAsk } from '@/test/ask-stream'
import userEvent from '@testing-library/user-event'
import { fireEvent, render, screen, waitFor, within } from '@/test/test-utils'
import { StudioView } from '@/components/views/StudioView'
import type { ArtifactLoad, EditPreview } from '@/lib/bindings'
import { useAgentStore } from '@/store/agent-store'
import { useStudioStore } from '@/store/studio-store'

const { mockClient } = vi.hoisted(() => ({
  mockClient: {
    ask: vi.fn(),
    loadArtifact: vi.fn(),
    setStudioOpenArtifact: vi
      .fn()
      .mockResolvedValue({ status: 'ok', data: null }),
    saveArtifact: vi.fn(),
    validateArtifact: vi.fn(),
    listPushTargets: vi.fn(),
    listCapabilities: vi.fn(),
    listAgents: vi.fn(),
    getWorkflows: vi.fn(),
    listTools: vi.fn(),
    listPolicies: vi.fn(),
    supports: { attachments: false },
    subscribe: vi.fn(() => ({
      ready: Promise.resolve(),
      unsubscribe: vi.fn(),
    })),
  },
}))

// jsdom has no layout, so the resize handle would catch every pointer
vi.mock('@/components/ui/resizable', () => ({
  ResizablePanelGroup: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
  ResizablePanel: ({ children }: { children: React.ReactNode }) => (
    <div>{children}</div>
  ),
  ResizableHandle: () => null,
}))

vi.mock('@/lib/client', () => ({
  client: mockClient,
  isDesktop: () => false,
}))

const CAP = `---
id: code-review.python
name: Python review
version: 1.2.3
policy:
  sla_seconds: 300
---

Review it.
`

const capability = (content = CAP): ArtifactLoad => ({
  artifact: { kind: 'capability', id: 'code-review.python' },
  content,
  parsed: {},
  path: '/repo/.nandi/capabilities/code-review.python.yaml',
})

const ok = <T,>(data: T) => ({ status: 'ok' as const, data })

// The ask box streams; answers come from the `ask` mock
streamAsksThroughAsk(mockClient)

describe('Studio split view', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    useAgentStore.setState(useAgentStore.getInitialState())
    useAgentStore.getState().setStudio([])
    useStudioStore.setState(useStudioStore.getInitialState())
    mockClient.listCapabilities.mockResolvedValue(
      ok([
        {
          definition: { id: 'code-review.python', name: 'Python review' },
          source: 'local',
        },
      ])
    )
    mockClient.listAgents.mockResolvedValue(
      ok([{ id: 'writer', name: 'Writer', description: '' }])
    )
    mockClient.getWorkflows.mockResolvedValue(ok([]))
    mockClient.listPolicies.mockResolvedValue(
      ok([
        { id: 'broken', name: 'broken', valid: false },
        { id: 'no-shell', name: 'No shell', valid: true },
      ])
    )
    mockClient.listTools.mockResolvedValue(
      ok([
        {
          definition: {
            id: 'builtin.shell',
            name: 'Shell',
            description: 'Runs commands',
          },
          enabled: true,
        },
      ])
    )
    mockClient.listPushTargets.mockResolvedValue(
      ok([
        { target: 'localOnly', available: true, branch: null, reason: null },
        {
          target: 'remoteBranch',
          available: true,
          branch: 'nandi/ada',
          reason: null,
        },
      ])
    )
    mockClient.loadArtifact.mockResolvedValue(ok(capability()))
    mockClient.validateArtifact.mockResolvedValue(
      ok({ ok: true, errors: [], warnings: [] })
    )
  })

  it("won't save a draft that wouldn't load, and says why", async () => {
    mockClient.validateArtifact.mockResolvedValue(
      ok({
        ok: false,
        errors: ["Not a valid capability: missing 'version'"],
        warnings: [],
      })
    )
    useStudioStore.getState().select('capability', 'code-review.python')
    render(<StudioView conversationId={null} />)

    const raw = await screen.findByLabelText('Raw file')
    fireEvent.change(raw, { target: { value: 'id: broken\n' } })
    expect(
      await screen.findByText("Not a valid capability: missing 'version'")
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled()
    expect(mockClient.validateArtifact).toHaveBeenCalledWith(
      { kind: 'capability', id: 'code-review.python' },
      'id: broken\n'
    )
  })

  it('shows warnings but still saves', async () => {
    mockClient.validateArtifact.mockResolvedValue(
      ok({
        ok: true,
        errors: [],
        warnings: ["Tool 'Glob' doesn't exist in Nandi; it will be ignored."],
      })
    )
    useStudioStore.getState().select('capability', 'code-review.python')
    render(<StudioView conversationId={null} />)

    const raw = await screen.findByLabelText('Raw file')
    fireEvent.change(raw, { target: { value: `${CAP}\nMore.\n` } })
    expect(
      await screen.findByText(/Tool 'Glob' doesn't exist/)
    ).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Save' })).toBeEnabled()
  })

  it('renders the chat and artifact panes side by side', () => {
    render(<StudioView conversationId={null} />)
    const chat = screen.getByRole('region', { name: 'Gateway chat' })
    const editor = screen.getByRole('region', { name: 'Artifact editor' })
    expect(within(chat).getByRole('textbox')).toBeInTheDocument()
    for (const tab of ['Capability', 'Agent', 'Workflow', 'Policy', 'Tool']) {
      expect(within(editor).getByRole('tab', { name: tab })).toBeInTheDocument()
    }
  })

  it('switches artifact types', async () => {
    const user = userEvent.setup()
    render(<StudioView conversationId={null} />)
    await user.click(screen.getByRole('tab', { name: 'Agent' }))
    expect(useStudioStore.getState().tab).toBe('agent')
    const picker = await screen.findByRole('combobox', { name: 'Open a Agent' })
    await waitFor(() =>
      expect(
        within(picker).getByRole('option', { name: 'Writer (writer)' })
      ).toBeInTheDocument()
    )

    await user.click(screen.getByRole('tab', { name: 'Tool' }))
    expect(await screen.findByText('Shell')).toBeInTheDocument()
  })

  it("lists NANDI.md, then the repo's policy files, marking broken ones", async () => {
    const user = userEvent.setup()
    render(<StudioView conversationId={null} />)
    await user.click(screen.getByRole('tab', { name: 'Policy' }))
    const picker = await screen.findByRole('combobox', {
      name: 'Open a Policy',
    })
    await waitFor(() =>
      expect(
        within(picker)
          .getAllByRole('option')
          .map(option => option.textContent)
      ).toEqual([
        'Choose…',
        'NANDI.md (gateway)',
        'broken (has errors)',
        'No shell (no-shell)',
      ])
    )

    await user.selectOptions(picker, 'broken')
    expect(useStudioStore.getState().selected.policy).toBe('broken')
  })

  it('updates the raw file from the form and saves through Studio', async () => {
    mockClient.saveArtifact.mockResolvedValue(
      ok({
        artifact: { kind: 'capability', id: 'code-review.python' },
        commit: 'abcdef123',
        pushedBranch: null,
        warnings: [],
      })
    )
    useStudioStore.getState().select('capability', 'code-review.python')
    const user = userEvent.setup()
    render(<StudioView conversationId={null} />)

    const sla = await screen.findByLabelText('SLA (seconds)')
    fireEvent.change(sla, { target: { value: '60' } })
    const raw = screen.getByLabelText('Raw file')
    expect((raw as HTMLTextAreaElement).value).toContain('  sla_seconds: 60\n')
    expect(screen.getByText('Unsaved')).toBeInTheDocument()

    await user.type(screen.getByLabelText('Commit message'), 'Faster SLA')
    await user.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() =>
      expect(mockClient.saveArtifact).toHaveBeenCalledWith({
        artifact: { kind: 'capability', id: 'code-review.python' },
        content: CAP.replace('sla_seconds: 300', 'sla_seconds: 60'),
        commitMessage: 'Faster SLA',
        pushTarget: 'localOnly',
      })
    )
  })

  it('asks before discarding, saying what is lost', async () => {
    useStudioStore.getState().select('capability', 'code-review.python')
    const user = userEvent.setup()
    render(<StudioView conversationId={null} />)
    fireEvent.change(await screen.findByLabelText('SLA (seconds)'), {
      target: { value: '60' },
    })
    await user.click(screen.getByRole('button', { name: 'Discard' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog).toHaveTextContent('1 line(s) added and 1 removed')
    // Still there until confirmed
    expect(screen.getByText('Unsaved')).toBeInTheDocument()
    await user.click(within(dialog).getByRole('button', { name: 'Discard' }))
    await waitFor(() =>
      expect(screen.queryByText('Unsaved')).not.toBeInTheDocument()
    )
  })

  it('asks before committing and pushing, with a suggested message', async () => {
    mockClient.saveArtifact.mockResolvedValue(
      ok({
        artifact: { kind: 'capability', id: 'code-review.python' },
        commit: 'abcdef123',
        pushedBranch: 'nandi/ada',
        warnings: [],
      })
    )
    useStudioStore.getState().select('capability', 'code-review.python')
    const user = userEvent.setup()
    render(<StudioView conversationId={null} />)
    fireEvent.change(await screen.findByLabelText('SLA (seconds)'), {
      target: { value: '60' },
    })
    await user.click(screen.getByRole('button', { name: /push/i }))
    await user.click(
      await screen.findByRole('menuitem', { name: /nandi\/ada/ })
    )
    const dialog = await screen.findByRole('alertdialog')
    expect(within(dialog).getByLabelText('Commit message')).toHaveValue(
      'Update the code-review.python capability'
    )
    expect(mockClient.saveArtifact).not.toHaveBeenCalled()
    await user.click(
      within(dialog).getByRole('button', { name: 'Commit and push' })
    )
    await waitFor(() =>
      expect(mockClient.saveArtifact).toHaveBeenCalledWith(
        expect.objectContaining({
          commitMessage: 'Update the code-review.python capability',
          pushTarget: 'remoteBranch',
        })
      )
    )
  })

  it('opens an approved edit in the editor', async () => {
    const preview: EditPreview = {
      artifact: { kind: 'capability', id: 'code-review.python' },
      before: CAP,
      after: CAP.replace('version: 1.2.3', 'version: 1.3.0'),
      diff: '@@\n-version: 1.2.3\n+version: 1.3.0\n',
      impact: [],
      versionBump: 'minor',
    }
    mockClient.ask.mockResolvedValue(ok({ kind: 'edit', data: preview }))
    // With a file open, the one input asks for a change to it
    useStudioStore.getState().select('capability', 'code-review.python')
    const user = userEvent.setup()
    render(<StudioView conversationId={null} />)
    expect(
      screen.queryByRole('group', { name: /mode/i })
    ).not.toBeInTheDocument()
    await screen.findByLabelText('Raw file')

    await user.type(
      within(screen.getByRole('region', { name: 'Gateway chat' })).getByRole(
        'textbox'
      ),
      'Bump the version{Enter}'
    )
    expect(mockClient.ask).toHaveBeenCalledWith(
      expect.objectContaining({
        context: expect.objectContaining({ currentView: 'studio' }),
      })
    )
    await user.click(await screen.findByRole('button', { name: 'Apply' }))

    const raw = await screen.findByLabelText('Raw file')
    expect((raw as HTMLTextAreaElement).value).toContain('version: 1.3.0')
    expect(useStudioStore.getState().pendingEdit).toBeNull()
    expect(screen.getByText('Unsaved')).toBeInTheDocument()
  })
})
