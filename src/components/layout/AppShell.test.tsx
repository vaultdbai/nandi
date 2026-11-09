import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { startAgentSync } from '@/lib/agent-sync'
import { useAgentStore } from '@/store/agent-store'
import { useNavigationStore, type View } from '@/store/navigation-store'
import { useNavTreeStore } from '@/store/nav-tree-store'
import { clearMockEventHandlers, emitMockEvent } from '@/test/mock-events'
import {
  T0,
  agentInfo,
  agentSnapshot,
  approvalMessage,
  dashboardSnapshot,
  run,
  userMessage,
} from '@/test/fixtures'
import { AppShell } from './AppShell'

const PERSONAL = agentInfo('personal', 'Personal Agent', { status: 'running' })
const WRITER = agentInfo('writer', 'Writer Agent')
const AGENTS = [PERSONAL, WRITER]
const writerIds = { agentId: 'writer', conversationId: 'agent:writer' }

const goTo = (view: View) =>
  act(() => useNavigationStore.getState().navigate(view))
const currentView = () => useNavigationStore.getState().current

let stopSync: () => void

async function renderShell() {
  stopSync = startAgentSync()
  render(<AppShell />)
  const nav = screen.getByRole('navigation', { name: 'Workspace' })
  // Boot snapshots (agents, dashboard, Studio) have loaded
  await vi.waitFor(() =>
    expect(useAgentStore.getState().agents.writer).toBeDefined()
  )
  return nav
}

/** jsdom gives every element a 0x0 box, so resizable panels eat clicks on
 * the composer; focus it directly. */
function composer() {
  const input = screen.getByRole('textbox')
  input.focus()
  return input
}

describe('AppShell', () => {
  beforeEach(() => {
    useAgentStore.setState(useAgentStore.getInitialState())
    useNavigationStore.setState(useNavigationStore.getInitialState())
    useNavTreeStore.setState({ expanded: [] })
    clearMockEventHandlers()
    vi.mocked(commands.listAgents).mockResolvedValue({
      status: 'ok',
      data: AGENTS,
    })
    vi.mocked(commands.getAgentSnapshot).mockImplementation(agentId =>
      Promise.resolve({
        status: 'ok',
        data: agentSnapshot(
          AGENTS.find(a => a.id === agentId) ?? null,
          agentId
        ),
      })
    )
  })

  afterEach(() => stopSync())

  describe('sidebar', () => {
    it('has the seven pages, and the Agents page lists agents with status', async () => {
      const user = userEvent.setup()
      const nav = await renderShell()

      for (const section of [
        'Home',
        'Agents',
        'Automations',
        'Library',
        'Network',
        'Ask',
        'Settings',
      ]) {
        expect(within(nav).getByRole('button', { name: section })).toBeVisible()
      }
      // Agents show in the sidebar once Agents is open
      expect(
        within(nav).queryByRole('button', { name: /Personal Agent/ })
      ).toBeNull()
      await user.click(within(nav).getByRole('button', { name: 'Agents' }))
      expect(
        await within(nav).findByRole('button', { name: 'Personal Agent' })
      ).toBeVisible()
      const agent = (
        await within(screen.getByRole('main')).findByText('Personal Agent')
      ).closest('li') as HTMLElement
      expect(within(agent).getByText('Running')).toBeVisible()
      // Its "⋯" is always there, not only on hover
      expect(
        within(agent).getByRole('button', {
          name: 'Actions for Personal Agent',
        })
      ).toBeVisible()
    })

    it('reflects a background run on the agent’s card', async () => {
      goTo({ kind: 'agents' })
      await renderShell()

      act(() =>
        emitMockEvent('agent://run', {
          ...writerIds,
          run: run('r1', 'writer', { status: 'awaitingApproval' }),
        })
      )

      const writer = (
        await within(screen.getByRole('main')).findByText('Writer Agent')
      ).closest('li') as HTMLElement
      expect(
        await within(writer).findByText('Waiting for your approval')
      ).toBeVisible()
    })

    it('opens a new chat, created with its first message', async () => {
      const user = userEvent.setup()
      const nav = await renderShell()

      // The sidebar's Ask button opens the conversation list
      await user.click(within(nav).getByRole('button', { name: 'Ask' }))
      // From the keyboard: in jsdom the resize handle claims pointer presses
      ;(await screen.findByRole('button', { name: 'New' })).focus()
      await user.keyboard('{Enter}')
      await user.click(await screen.findByRole('menuitem', { name: 'Chat' }))

      expect(commands.createStudioConversation).not.toHaveBeenCalled()
      expect(currentView()).toEqual({
        kind: 'studio',
        conversationId: null,
        draft: true,
      })
      // An example ask sends the first message, which creates the chat
      await user.click(
        await screen.findByRole('button', { name: "What's waiting for me?" })
      )
      expect(commands.createStudioConversation).toHaveBeenCalled()
      expect(currentView()).toEqual({
        kind: 'studio',
        conversationId: 'studio-new',
      })
      // Later tests read the first message sent
      vi.mocked(commands.sendStudioMessage).mockClear()
    })
  })

  describe('background execution', () => {
    it('keeps applying events while the user is on another view', async () => {
      goTo({ kind: 'agent', agentId: 'writer', tab: 'chat' })
      await renderShell()

      goTo({ kind: 'home' })
      act(() => {
        emitMockEvent('agent://message', {
          ...writerIds,
          message: userMessage('u1', 'Summarize the report'),
        })
        emitMockEvent('agent://token', {
          ...writerIds,
          runId: 'r1',
          messageId: 'm1',
          offset: 0,
          text: 'Here is the **summary**',
          createdAt: T0,
        })
      })

      goTo({ kind: 'agent', agentId: 'writer', tab: 'chat' })

      expect(screen.getByText('Summarize the report')).toBeInTheDocument()
      expect(screen.getByText('summary').tagName).toBe('STRONG')
    })
  })

  describe('home', () => {
    it('shows what to do first on a fresh home', async () => {
      vi.mocked(commands.getDashboardSnapshot).mockResolvedValue({
        status: 'ok',
        data: dashboardSnapshot(),
      })
      await renderShell()

      // Nothing has happened yet: what to do first, not empty cards
      expect(await screen.findByText('Get started')).toBeInTheDocument()
    })

    it('opens a recent result as a run and re-runs it', async () => {
      const user = userEvent.setup()
      const completed = run('r9', 'writer', {
        status: 'completed',
        input: 'Weekly summary',
        summary: 'Five things happened.',
        startedAt: Date.now() - 120_000,
        endedAt: Date.now() - 60_000,
      })
      vi.mocked(commands.getDashboardSnapshot).mockResolvedValue({
        status: 'ok',
        data: dashboardSnapshot({
          recentOutputs: [{ run: completed, agentName: 'Writer Agent' }],
        }),
      })
      vi.mocked(commands.listAllRuns).mockResolvedValue({
        status: 'ok',
        data: { rows: [completed], total: 1 },
      })
      vi.mocked(commands.getRunDetail).mockResolvedValue({
        status: 'ok',
        data: {
          run: completed,
          agentName: 'Writer Agent',
          messages: [userMessage('u9', 'Weekly summary', 'r9')],
          events: [],
        },
      })
      await renderShell()

      // Finished work is in the work tray, on every page
      const done = screen.getByTestId('tray-done')
      await user.click(await within(done).findByText('Weekly summary'))

      expect(
        await screen.findByRole('heading', {
          level: 1,
          name: /^Writer Agent · Weekly summary · /,
        })
      ).toBeInTheDocument()
      expect(commands.getRunDetail).toHaveBeenCalledWith('r9')

      await user.click(screen.getByRole('button', { name: 'Re-run' }))

      await vi.waitFor(() =>
        expect(commands.sendMessage).toHaveBeenCalledWith(
          'writer',
          'Weekly summary',
          [],
          expect.any(String)
        )
      )
      // Re-run stays on the run (task 9); the toast links to the agent's runs
      expect(currentView()).toMatchObject({ kind: 'run-detail', runId: 'r9' })
    })
  })

  describe('agent view', () => {
    it('shows the snapshot transcript and approves via the command', async () => {
      const user = userEvent.setup()
      vi.mocked(commands.getAgentSnapshot).mockImplementation(agentId =>
        Promise.resolve({
          status: 'ok',
          data: agentSnapshot(
            AGENTS.find(a => a.id === agentId) ?? null,
            agentId,
            agentId === 'writer'
              ? {
                  messages: [
                    userMessage('u1', 'Check it'),
                    approvalMessage('a1'),
                  ],
                  currentRun: run('r1', 'writer', {
                    status: 'awaitingApproval',
                  }),
                  currentTurn: run('r1', 'writer', {
                    status: 'awaitingApproval',
                  }),
                }
              : {}
          ),
        })
      )
      goTo({ kind: 'agent', agentId: 'writer', focusMessageId: 'a1' })
      await renderShell()

      // The approval in plain words, the JSON behind Details
      expect(
        screen.getByText('Run a command: wc -w report.md')
      ).toBeInTheDocument()
      expect(
        screen.getByText('Waiting for approval: Do the thing')
      ).toBeInTheDocument()
      expect(document.querySelector('[data-focused]')).not.toBeNull()
      // Mid-run: a new message waits in the agent's queue
      expect(
        screen.getByPlaceholderText('Queue a message for Writer Agent…')
      ).toBeEnabled()

      await user.click(screen.getByRole('button', { name: 'Approve' }))
      expect(commands.approveAction).toHaveBeenCalledWith(
        'writer',
        'a1',
        'approved'
      )

      // The decision is shown once Rust confirms it
      act(() =>
        emitMockEvent('agent://approval_resolved', {
          ...writerIds,
          runId: 'r1',
          approvalId: 'a1',
          status: 'approved',
        })
      )
      expect(screen.getByText('Approved')).toBeInTheDocument()
    })

    it('stops a run', async () => {
      const user = userEvent.setup()
      goTo({ kind: 'agent', agentId: 'writer', tab: 'chat' })
      await renderShell()
      act(() =>
        emitMockEvent('agent://run', { ...writerIds, run: run('r1', 'writer') })
      )

      await user.click(screen.getByRole('button', { name: 'Stop' }))

      expect(commands.cancelTurn).toHaveBeenCalledWith('writer', 'r1')
    })

    it('controls the container and shows its status', async () => {
      const user = userEvent.setup()
      vi.mocked(commands.getAgentSnapshot).mockResolvedValue({
        status: 'ok',
        data: agentSnapshot(WRITER, 'writer', {
          container: { status: 'idle', error: null },
        }),
      })
      goTo({ kind: 'agent', agentId: 'writer', tab: 'chat' })
      await renderShell()

      // The container lives in the page's "⋯", not the header
      const menu = await screen.findByRole('button', {
        name: 'Actions for Writer Agent',
      })
      // Keyboard: jsdom's pointer events don't open Radix menus here
      const openMenu = async () => {
        menu.focus()
        await user.keyboard('{Enter}')
      }
      await openMenu()
      expect(await screen.findByText('Container: stopped')).toBeInTheDocument()
      expect(
        screen.queryByRole('menuitem', { name: 'Stop container' })
      ).toBeNull()

      vi.mocked(commands.getAgentSnapshot).mockResolvedValue({
        status: 'ok',
        data: agentSnapshot(WRITER, 'writer', {
          container: { status: 'running', error: null },
        }),
      })
      await user.click(
        screen.getByRole('menuitem', { name: 'Start container' })
      )
      expect(commands.startAgent).toHaveBeenCalledWith('writer')

      await openMenu()
      expect(await screen.findByText('Container: running')).toBeInTheDocument()
      // Restarting a running container asks first
      await user.click(
        screen.getByRole('menuitem', { name: 'Restart container' })
      )
      expect(
        await screen.findByText('Restart the container?')
      ).toBeInTheDocument()
      expect(commands.restartAgent).not.toHaveBeenCalled()
    })

    it('opens on Overview, with Chat, Runs and Settings tabs', async () => {
      goTo({ kind: 'agent', agentId: 'writer' })
      await renderShell()

      expect(
        await screen.findByRole('tab', { name: 'Overview', selected: true })
      ).toBeInTheDocument()
      for (const tab of ['Chat', 'Runs', 'Settings']) {
        expect(screen.getByRole('tab', { name: tab })).toBeInTheDocument()
      }
      // No skill.md path in the header
      expect(screen.queryByText(/skill\.md/)).toBeNull()
    })

    it('shows a not-found state for unknown agents', async () => {
      goTo({ kind: 'agent', agentId: 'ghost' })
      await renderShell()

      expect(
        await screen.findByText('This agent no longer exists')
      ).toBeInTheDocument()
      // No folder path in the message
      expect(screen.queryByText(/~\/\.nandi/)).toBeNull()
      // No message box for an agent that isn't there; a way back instead
      expect(screen.queryByRole('button', { name: 'Send message' })).toBeNull()
      expect(
        screen.getByRole('button', { name: 'Back to Agents' })
      ).toBeInTheDocument()
    })
  })

  describe('LLM key notice', () => {
    it('asks for a key on agent views while no model can be called', async () => {
      const user = userEvent.setup()
      vi.mocked(commands.getDefaultModel).mockResolvedValue({
        status: 'ok',
        data: {
          defaultModel: 'claude:claude-sonnet-5',
          effectiveModel: null,
          claudeSignIn: null,
          trial: null,
        },
      })
      goTo({ kind: 'agent', agentId: 'writer', tab: 'chat' })
      await renderShell()

      const notice = await screen.findByRole('note')
      expect(notice).toHaveTextContent('No model connected')

      await user.click(
        within(notice).getByRole('button', { name: 'Connect a model' })
      )
      // Straight to the keys, on the Settings page
      expect(useNavigationStore.getState().current).toEqual({
        kind: 'settings',
        section: 'models',
      })
      vi.mocked(commands.getDefaultModel).mockResolvedValue({
        status: 'ok',
        data: {
          defaultModel: 'claude:claude-sonnet-5',
          effectiveModel: 'claude:claude-sonnet-5',
          claudeSignIn: null,
          trial: null,
        },
      })
    })

    it('stays hidden when another provider has a key', async () => {
      vi.mocked(commands.getDefaultModel).mockResolvedValue({
        status: 'ok',
        data: {
          defaultModel: 'deepseek:deepseek-flash',
          effectiveModel: 'gpt:gpt-5',
          claudeSignIn: null,
          trial: null,
        },
      })
      goTo({ kind: 'agent', agentId: 'writer', tab: 'chat' })
      await renderShell()
      await screen.findAllByText(/Writer Agent/)
      expect(screen.queryByRole('note')).not.toBeInTheDocument()
      vi.mocked(commands.getDefaultModel).mockResolvedValue({
        status: 'ok',
        data: {
          defaultModel: 'claude:claude-sonnet-5',
          effectiveModel: 'claude:claude-sonnet-5',
          claudeSignIn: null,
          trial: null,
        },
      })
    })
  })

  describe('docked input', () => {
    it('sends to the agent and shows the message immediately', async () => {
      const user = userEvent.setup()
      goTo({ kind: 'agent', agentId: 'writer', tab: 'chat' })
      await renderShell()

      const input = composer()
      expect(input).toHaveAttribute('placeholder', 'Message Writer Agent…')
      await user.keyboard('Line one{Shift>}{Enter}{/Shift}Line two{Enter}')

      expect(input).toHaveValue('')
      expect(commands.sendMessage).toHaveBeenCalledWith(
        'writer',
        'Line one\nLine two',
        [],
        expect.any(String)
      )
      expect(screen.getByText(/Line one/)).toBeInTheDocument()
    })

    it('attaches files picked by Rust, sends their ids, and discards removed ones', async () => {
      const user = userEvent.setup()
      vi.mocked(commands.pickAttachments).mockResolvedValueOnce({
        status: 'ok',
        data: [
          { id: 'att-1', name: 'brief.pdf', kind: 'file', sizeBytes: 2048 },
          { id: 'att-2', name: 'photo.png', kind: 'image', sizeBytes: 4096 },
        ],
      })
      goTo({ kind: 'agent', agentId: 'writer', tab: 'chat' })
      await renderShell()

      await user.click(screen.getByRole('button', { name: 'Attach files' }))
      expect(commands.pickAttachments).toHaveBeenCalledWith('file')
      const chips = await screen.findByRole('list', { name: 'Attachments' })

      await user.click(screen.getByRole('button', { name: 'Remove photo.png' }))
      expect(commands.discardAttachment).toHaveBeenCalledWith('att-2')
      expect(within(chips).queryByText('photo.png')).toBeNull()

      await user.click(screen.getByRole('button', { name: 'Send message' }))
      expect(commands.sendMessage).toHaveBeenCalledWith(
        'writer',
        '',
        ['att-1'],
        expect.any(String)
      )
    })

    it("Home's Ask Nandi sends the text to Studio", async () => {
      const user = userEvent.setup()
      await renderShell()

      const ask = screen.getByRole('textbox', { name: 'Ask Nandi' })
      ask.focus()
      await user.keyboard('Plan my week{Enter}')

      await vi.waitFor(() =>
        expect(
          vi.mocked(commands.sendStudioMessage).mock.calls[0]?.slice(0, 2)
        ).toEqual(['studio-new', 'Plan my week'])
      )
      expect(currentView()).toEqual({
        kind: 'studio',
        conversationId: 'studio-new',
      })
    })

    it('has no docked bar on pages without a conversation', async () => {
      goTo({ kind: 'capability-detail', id: 'code-review.python' })
      await renderShell()
      expect(
        screen.queryByPlaceholderText(/Ask about code-review\.python/)
      ).toBeNull()
      expect(screen.queryByRole('radio', { name: 'Chat' })).toBeNull()
    })

    it('on an agent page, Ask asks about the agent', async () => {
      const user = userEvent.setup()
      goTo({ kind: 'agent', agentId: 'writer', tab: 'chat' })
      await renderShell()

      const sent = vi.mocked(commands.sendMessage).mock.calls.length
      await user.click(screen.getByRole('radio', { name: 'Ask' }))
      const input = composer()
      expect(input).toHaveAttribute(
        'placeholder',
        'Ask about Writer Agent, or change it…'
      )
      await user.keyboard('Rename it to Scribe{Enter}')
      await vi.waitFor(() =>
        expect(commands.ask).toHaveBeenCalledWith(
          expect.objectContaining({
            context: {
              currentView: 'agent:writer',
              focusedArtifact: { kind: 'agent', id: 'writer' },
              recentRuns: [],
            },
          })
        )
      )
      expect(vi.mocked(commands.sendMessage).mock.calls).toHaveLength(sent)
    })
  })

  describe('studio', () => {
    it('lists conversations in the Studio view, not the sidebar', async () => {
      vi.mocked(commands.getStudioConversations).mockResolvedValue({
        status: 'ok',
        data: [
          {
            id: 's1',
            title: 'Tighten the Researcher prompt',
            createdAt: T0,
            updatedAt: T0,
            lastMessage: 'Prefer papers',
          },
        ],
      })
      goTo({ kind: 'studio', conversationId: null })
      const nav = await renderShell()

      // In the Studio view; the sidebar lists it only once Ask is open
      expect(
        await within(screen.getByRole('main')).findByRole('button', {
          name: /Tighten the Researcher prompt/,
        })
      ).toBeVisible()
      expect(
        within(nav).queryByText('Tighten the Researcher prompt')
      ).toBeNull()
    })
  })
})
