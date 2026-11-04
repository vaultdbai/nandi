import { describe, expect, it, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@/test/test-utils'
import type { PaletteResult } from '@/lib/bindings'
import { registerCommands } from '@/lib/commands/registry'
import { navigationCommands } from '@/lib/commands/navigation-commands'
import { useKeyboardShortcuts } from '@/hooks/use-keyboard-shortcuts'
import { useCommandContext } from '@/hooks/use-command-context'
import { useNavigationStore } from '@/store/navigation-store'
import { useUIStore } from '@/store/ui-store'
import { CommandPalette } from './CommandPalette'
import { clearRecents, loadRecents } from './recents'

const { mockClient } = vi.hoisted(() => ({
  mockClient: {
    searchPalette: vi.fn(),
    executePaletteAction: vi.fn(),
    createStudioConversation: vi.fn(),
    mascotPhrase: vi.fn(),
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

const result = (
  kind: string,
  id: string,
  name: string,
  extra: Partial<PaletteResult> = {}
): PaletteResult => ({
  kind,
  id,
  name,
  description: `${name} description`,
  score: 50,
  args: null,
  navigate: null,
  ...extra,
})

const ACTIONS = [
  result('action', 'pause_all', 'Pause all agents'),
  result('action', 'open_view', 'Go to Library', {
    args: { view: { kind: 'library', tab: 'browse' } },
    navigate: { kind: 'library', tab: 'browse' },
  }),
]

const EVERYTHING = [
  ...ACTIONS,
  result('agent', 'portfolio', 'Portfolio Analyst', {
    navigate: { kind: 'agent', agentId: 'portfolio' },
  }),
  result('workflow', 'daily', 'Daily Report', {
    navigate: { kind: 'workflow', workflowId: 'daily' },
  }),
  result('run', 'r1', 'Portfolio Analyst · run r1', {
    navigate: { kind: 'run-detail', runId: 'r1' },
  }),
  result('package', 'risk', 'Risk Monitor', {
    navigate: { kind: 'library', tab: 'browse' },
  }),
  result('tool', 'github', 'GitHub', {
    navigate: { kind: 'library', tab: 'browse', filter: 'tools' },
  }),
]

function search(query: string) {
  const q = query.trim()
  if (q === '') return [result('action', 'open_view', 'Go to Home')]
  if (q.startsWith('>')) return ACTIONS
  return EVERYTHING
}

const input = () => screen.getByRole('combobox')
const current = () => useNavigationStore.getState().current

function openPalette() {
  act(() => useUIStore.getState().setCommandPaletteOpen(true))
}

describe('CommandPalette', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    clearRecents()
    useUIStore.setState(useUIStore.getInitialState())
    useNavigationStore.setState(useNavigationStore.getInitialState())
    mockClient.searchPalette.mockImplementation((q: string) =>
      Promise.resolve({ status: 'ok', data: search(q) })
    )
    mockClient.executePaletteAction.mockResolvedValue({
      status: 'ok',
      data: { message: 'Paused 2 agent(s)', navigate: null },
    })
    registerCommands(navigationCommands)
  })

  it('renders nothing while closed', () => {
    render(<CommandPalette />)
    expect(screen.queryByRole('combobox')).not.toBeInTheDocument()
  })

  it('shows suggestions for an empty query', async () => {
    render(<CommandPalette />)
    openPalette()
    expect(await screen.findByText('Go to Home')).toBeInTheDocument()
    expect(mockClient.searchPalette).toHaveBeenCalledWith('')
  })

  it('renders results grouped by kind', async () => {
    const user = userEvent.setup()
    render(<CommandPalette />)
    openPalette()
    await user.type(input(), 'port')

    expect(await screen.findByText('Portfolio Analyst')).toBeInTheDocument()
    const groups = screen.getAllByRole('group')
    const labels = groups.map(group => group.getAttribute('aria-label'))
    expect(labels).toEqual([
      'Ask Nandi',
      'Actions',
      'Agents',
      'Workflows',
      'Runs',
      'Packages',
      'Tools',
    ])
    const agents = groups[2]
    expect(agents).toBeDefined()
    expect(
      within(agents as HTMLElement).getByRole('option', {
        name: /Portfolio Analyst/,
      })
    ).toBeInTheDocument()
    expect(mockClient.searchPalette).toHaveBeenLastCalledWith('port')
  })

  it('shows only actions in > mode, including local commands', async () => {
    const user = userEvent.setup()
    render(<CommandPalette />)
    openPalette()
    await user.type(input(), '>')

    expect(await screen.findByText('Pause all agents')).toBeInTheDocument()
    await waitFor(() =>
      expect(mockClient.searchPalette).toHaveBeenLastCalledWith('>')
    )
    const groups = screen.getAllByRole('group')
    expect(groups.map(g => g.getAttribute('aria-label'))).toEqual(['Actions'])
    // The local command registry (sidebars, preferences) is listed too
    expect(screen.getByText('Open Settings')).toBeInTheDocument()
    expect(screen.queryByText('Portfolio Analyst')).not.toBeInTheDocument()
  })

  it('runs the highlighted action on Enter', async () => {
    const user = userEvent.setup()
    render(<CommandPalette />)
    openPalette()
    await user.type(input(), '> pause')
    await screen.findByText('Pause all agents')
    await waitFor(() =>
      expect(screen.getAllByRole('option')[0]).toHaveTextContent(
        'Pause all agents'
      )
    )

    await user.keyboard('{Enter}')
    await waitFor(() =>
      expect(mockClient.executePaletteAction).toHaveBeenCalledWith(
        'pause_all',
        null
      )
    )
    expect(useUIStore.getState().commandPaletteOpen).toBe(false)
    expect(loadRecents()[0]?.id).toBe('pause_all')
  })

  it('answers the hidden phrase without running anything', async () => {
    mockClient.mascotPhrase.mockResolvedValue({
      status: 'ok',
      data: { line: 'The bull carries on.' },
    })
    const user = userEvent.setup()
    render(<CommandPalette />)
    openPalette()
    // Tests don't ship; the words are only spelled out here
    await user.type(input(), 'Appa yip yip!')
    await user.keyboard('{Enter}')
    await waitFor(() =>
      expect(mockClient.mascotPhrase).toHaveBeenCalledWith('Appa yip yip!')
    )
    expect(mockClient.executePaletteAction).not.toHaveBeenCalled()
    expect(mockClient.createStudioConversation).not.toHaveBeenCalled()
    expect(useUIStore.getState().commandPaletteOpen).toBe(false)
  })

  it('navigates to the outcome view with the arrow keys and Enter', async () => {
    mockClient.executePaletteAction.mockResolvedValue({
      status: 'ok',
      data: { message: null, navigate: { kind: 'library', tab: 'browse' } },
    })
    const user = userEvent.setup()
    render(<CommandPalette />)
    openPalette()
    await user.type(input(), '>library')
    await screen.findByText('Go to Library')
    const options = () => screen.getAllByRole('option')
    const index = options().findIndex(o =>
      o.textContent?.includes('Go to Library')
    )
    for (let i = 0; i < index; i++) await user.keyboard('{ArrowDown}')
    expect(options()[index]).toHaveAttribute('aria-selected', 'true')

    await user.keyboard('{Enter}')
    await waitFor(() =>
      expect(current()).toEqual({ kind: 'library', tab: 'browse' })
    )
    expect(mockClient.executePaletteAction).toHaveBeenCalledWith('open_view', {
      view: { kind: 'library', tab: 'browse' },
    })
  })

  it('offers "Ask Nandi: <text>" first and sends it to Studio', async () => {
    mockClient.createStudioConversation.mockResolvedValue({
      status: 'error',
      error: 'no studio in this test',
    })
    const user = userEvent.setup()
    render(<CommandPalette />)
    openPalette()
    await user.type(input(), 'what failed today')

    const first = (await screen.findAllByRole('option'))[0]
    expect(first).toHaveTextContent('Ask Nandi: what failed today')
    await user.keyboard('{Enter}')
    await waitFor(() =>
      expect(mockClient.createStudioConversation).toHaveBeenCalled()
    )
    expect(useUIStore.getState().commandPaletteOpen).toBe(false)
    // Asks aren't kept as recents
    expect(loadRecents()).toEqual([])
  })

  it('opens entity results directly', async () => {
    const user = userEvent.setup()
    render(<CommandPalette />)
    openPalette()
    await user.type(input(), 'port')
    await user.click(await screen.findByText('Portfolio Analyst'))
    expect(current()).toEqual({ kind: 'agent', agentId: 'portfolio' })
    expect(mockClient.executePaletteAction).not.toHaveBeenCalled()
  })

  it('closes on Escape', async () => {
    const user = userEvent.setup()
    render(<CommandPalette />)
    openPalette()
    await screen.findByRole('combobox')
    await user.keyboard('{Escape}')
    expect(useUIStore.getState().commandPaletteOpen).toBe(false)
  })
})

function WithShortcuts() {
  useKeyboardShortcuts(useCommandContext())
  return <CommandPalette />
}

describe('Cmd/Ctrl+K', () => {
  beforeEach(() => {
    useUIStore.setState(useUIStore.getInitialState())
    mockClient.searchPalette.mockResolvedValue({ status: 'ok', data: [] })
  })

  it.each([
    ['Ctrl', { ctrlKey: true }],
    ['Cmd', { metaKey: true }],
  ])('%s+K toggles the palette', async (_name, modifier) => {
    render(<WithShortcuts />)
    expect(screen.queryByRole('combobox')).not.toBeInTheDocument()

    fireEvent.keyDown(document, { key: 'k', ...modifier })
    expect(await screen.findByRole('combobox')).toHaveFocus()

    fireEvent.keyDown(document, { key: 'k', ...modifier })
    await waitFor(() =>
      expect(screen.queryByRole('combobox')).not.toBeInTheDocument()
    )
  })
})
