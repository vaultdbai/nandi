import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { RelayStatus } from '@/lib/bindings'
import { useAgentStore } from '@/store/agent-store'
import { useNavigationStore, type View } from '@/store/navigation-store'
import { useNavTreeStore } from '@/store/nav-tree-store'
import { useUIStore } from '@/store/ui-store'
import { Sidebar } from './Sidebar'

const navButton = (name: string | RegExp) =>
  within(screen.getByRole('navigation')).getByRole('button', { name })

const current = () => useNavigationStore.getState().current

const relay = (relayUrl: string | null, connected: boolean): RelayStatus => ({
  connected,
  state: connected ? 'connected' : 'disconnected',
  reason: null,
  instanceId: 'me',
  relayUrl,
  relayUrlSource: relayUrl ? 'config' : null,
  agentsOnline: 0,
  instancesOnline: 0,
  lastConnectedAt: null,
  lastHeartbeatAt: null,
})

const ITEMS = [
  'Home',
  'Ask',
  'My notes',
  'Agents',
  'Automations',
  'Library',
  'Network',
  'Settings',
] as const

describe('Sidebar', () => {
  beforeEach(() => {
    useAgentStore.setState(useAgentStore.getInitialState())
    useNavigationStore.setState(useNavigationStore.getInitialState())
    useUIStore.setState(useUIStore.getInitialState())
    useNavTreeStore.setState({ expanded: [] })
    localStorage.clear()
    Object.assign(commands, {
      getRelayStatus: vi
        .fn()
        .mockResolvedValue({ status: 'ok', data: relay(null, false) }),
      listOnlineAgents: vi.fn().mockResolvedValue({ status: 'ok', data: [] }),
    })
  })

  it('has eight items, in order, closed at first', () => {
    render(<Sidebar />)
    const buttons = within(screen.getByRole('navigation')).getAllByRole(
      'button'
    )
    expect(buttons.map(b => b.textContent)).toEqual([...ITEMS])
    // Home opens to its Inbox and All activity; Settings has nothing under it
    expect(navButton('Home')).toHaveAttribute('aria-expanded', 'false')
    expect(navButton('Automations')).toHaveAttribute('aria-expanded', 'false')
    expect(navButton('Settings')).not.toHaveAttribute('aria-expanded')
  })

  it.each<[string, View]>([
    ['Home', { kind: 'home' }],
    ['Agents', { kind: 'agents' }],
    ['Automations', { kind: 'automations' }],
    ['Library', { kind: 'library' }],
    ['Network', { kind: 'network' }],
    ['Ask', { kind: 'studio', conversationId: null }],
    ['My notes', { kind: 'notes' }],
    ['Settings', { kind: 'settings' }],
  ])('%s opens its page', async (label, view) => {
    const user = userEvent.setup()
    render(<Sidebar />)
    await user.click(navButton(label))
    expect(current()).toEqual(view)
    expect(navButton(label)).toHaveAttribute('aria-current', 'page')
  })

  it('opens a page’s branch, with its live list', async () => {
    vi.mocked(commands.listAgents).mockResolvedValue({
      status: 'ok',
      data: [
        {
          id: 'writer',
          name: 'Writer',
          description: '',
          skillPath: '',
          status: 'running',
          currentTask: null,
          origin: { kind: 'local' },
          builtin: false,
        },
      ],
    })
    const user = userEvent.setup()
    render(<Sidebar />)
    await user.click(navButton('Agents'))
    expect(navButton('Agents')).toHaveAttribute('aria-expanded', 'true')
    await user.click(
      await within(screen.getByRole('navigation')).findByRole('button', {
        name: 'Writer',
      })
    )
    expect(current()).toEqual({ kind: 'agent', agentId: 'writer' })
    // The agent is the current page, under Agents; nothing under it
    expect(navButton('Writer')).toHaveAttribute('aria-current', 'page')
    expect(navButton('Writer')).not.toHaveAttribute('aria-expanded')
    expect(navButton('Agents')).not.toHaveAttribute('aria-current')
  })

  it('has New chat and All chats under Ask', async () => {
    const user = userEvent.setup()
    render(<Sidebar />)
    await user.click(navButton('Ask'))
    // New chat opens an empty chat; nothing is created yet
    await user.click(navButton(/New chat/i))
    expect(current()).toEqual({
      kind: 'studio',
      conversationId: null,
      draft: true,
    })
    expect(navButton(/New chat/i)).toHaveAttribute('aria-current', 'page')
    expect(commands.createStudioConversation).not.toHaveBeenCalled()
    await user.click(navButton(/^All chats/))
    expect(current()).toEqual({ kind: 'studio', conversationId: null })
  })

  it('opens the branches above the current page by itself, and remembers them', async () => {
    render(<Sidebar />)
    act(() =>
      useNavigationStore
        .getState()
        .navigate({ kind: 'automations', tab: 'triggers' })
    )
    expect(
      await screen.findByRole('button', { name: 'Triggers' })
    ).toHaveAttribute('aria-current', 'page')
    expect(navButton('Automations')).toHaveAttribute('aria-expanded', 'true')
    expect(
      JSON.parse(localStorage.getItem('nav-tree-expanded') ?? '[]')
    ).toEqual(['automations'])
  })

  it('opens and closes branches with Right and Left', async () => {
    const user = userEvent.setup()
    render(<Sidebar />)
    navButton('Automations').focus()
    await user.keyboard('{ArrowRight}')
    expect(navButton('Automations')).toHaveAttribute('aria-expanded', 'true')
    await user.keyboard('{ArrowRight}')
    expect(navButton('Workflows')).toHaveFocus()
    await user.keyboard('{ArrowDown}')
    expect(navButton('Triggers')).toHaveFocus()
    await user.keyboard('{ArrowLeft}')
    expect(navButton('Automations')).toHaveFocus()
    await user.keyboard('{ArrowLeft}')
    expect(navButton('Automations')).toHaveAttribute('aria-expanded', 'false')
    // Moving around never navigates
    expect(current()).toEqual(useNavigationStore.getInitialState().current)
  })

  it('stays active on the pages under an item', async () => {
    render(<Sidebar />)
    act(() =>
      useNavigationStore.getState().navigate({ kind: 'agent', agentId: 'a' })
    )
    expect(navButton('Agents')).toHaveAttribute('aria-current', 'page')
    act(() =>
      useNavigationStore
        .getState()
        .navigate({ kind: 'member-profile', instanceId: 'i' })
    )
    expect(navButton('Network')).toHaveAttribute('aria-current', 'page')
    expect(navButton('Agents')).not.toHaveAttribute('aria-current')
  })

  it('moves between items with the arrow keys', async () => {
    const user = userEvent.setup()
    render(<Sidebar />)
    navButton('Home').focus()
    await user.keyboard('{ArrowDown}')
    expect(navButton('Ask')).toHaveFocus()
    await user.keyboard('{ArrowUp}{ArrowUp}')
    expect(navButton('Settings')).toHaveFocus()
    await user.keyboard('{Home}')
    expect(navButton('Home')).toHaveFocus()
    await user.keyboard('{End}{Enter}')
    expect(current()).toEqual({ kind: 'settings' })
  })

  it('counts the Inbox on Home, leaving failures out', async () => {
    const approval = (id: string) => ({
      id,
      agentId: 'writer',
      runId: 'r1',
      conversationId: 'agent:writer',
      tool: 'shell',
      args: {},
      createdAt: 1,
      expiresAt: null,
    })
    vi.mocked(commands.listPendingApprovals).mockResolvedValue({
      status: 'ok',
      data: [approval('a1'), approval('a2')],
    })
    vi.mocked(commands.listWork).mockResolvedValue({
      status: 'ok',
      data: [
        {
          id: 'w-failed',
          kind: 'workflow',
          title: 'Digest',
          state: 'failed',
          owner: null,
          progress: { completedSteps: 0, totalSteps: 1, percent: 0 },
          nextAction: null,
          workflowId: 'digest',
          rating: null,
          startedAt: Date.now(),
          updatedAt: Date.now(),
        },
      ],
    })
    render(<Sidebar />)
    expect(
      await screen.findByRole('button', { name: 'Home, 2 waiting' })
    ).toBeInTheDocument()
  })

  it('shows the connection as a dot on Network', async () => {
    vi.mocked(commands.getRelayStatus).mockResolvedValue({
      status: 'ok',
      data: relay('wss://relay.nandi.dev/relay', true),
    })
    Object.assign(commands, {
      getGatewayStatus: vi.fn().mockResolvedValue({
        status: 'ok',
        data: {
          enabled: true,
          registered: true,
          discoverable: true,
          card: null,
          activeTasks: 0,
          queuedTasks: 0,
          lastHeartbeatAt: Date.now(),
        },
      }),
    })
    render(<Sidebar />)
    expect(
      await screen.findByRole('img', {
        name: 'Connected · others can reach you',
      })
    ).toHaveAttribute('data-health', 'green')
  })

  it('is amber while connected but not listed, red when the relay refuses', async () => {
    vi.mocked(commands.getRelayStatus).mockResolvedValue({
      status: 'ok',
      data: relay('wss://relay.nandi.dev/relay', true),
    })
    Object.assign(commands, {
      getGatewayStatus: vi.fn().mockResolvedValue({
        status: 'ok',
        data: {
          enabled: false,
          registered: false,
          discoverable: false,
          card: null,
          activeTasks: 0,
          queuedTasks: 0,
          lastHeartbeatAt: null,
        },
      }),
    })
    const { unmount } = render(<Sidebar />)
    expect(
      await screen.findByRole('img', { name: 'Connected · not listed yet' })
    ).toHaveAttribute('data-health', 'amber')
    unmount()
    vi.mocked(commands.getRelayStatus).mockResolvedValue({
      status: 'ok',
      data: { ...relay('wss://relay.nandi.dev/relay', false), state: 'error' },
    })
    render(<Sidebar />)
    expect(
      await screen.findByRole('img', { name: "Can't reach the relay" })
    ).toHaveAttribute('data-health', 'red')
  })

  it('shows no dot without a network', async () => {
    render(<Sidebar />)
    await screen.findByRole('navigation')
    expect(screen.queryByTestId('network-dot')).toBeNull()
  })
})
