import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { agentManifest, packManifest } from '@/test/fixtures'
import { commands } from '@/lib/tauri-bindings'
import { ok } from '@/hooks/useSharedAgents.fixtures'
import type { ConnectorDetails } from '@/hooks/useConnectors'
import type { ConsentStatus, ToolEntry } from '@/lib/bindings'
import { useMarketplaceStore } from '@/store/marketplace-store'
import { useNavigationStore, type View } from '@/store/navigation-store'
import { LibraryPageView } from './LibraryView'

const writer = agentManifest('writer', 'Writer', { tags: ['drafting'] })
const research = packManifest('research', 'Research Pack', {
  installedVersion: '1.0.0',
})

const shell: ToolEntry = {
  definition: {
    id: 'shell',
    name: 'shell',
    description: 'Runs commands',
    source: { kind: 'builtin' },
    inputSchema: null,
    requiresApproval: true,
    costEstimate: null,
    latencyClass: 'instant',
    tags: [],
  },
  enabled: true,
  usageCount: 0,
}

const github = {
  id: 'github',
  name: 'GitHub',
  description: 'Reads pull requests',
  kind: 'mcp',
  availability: 'available',
  status: { installed: true, running: false, healthy: false, error: null },
  tools: [],
  config: {},
  secrets: [
    {
      key: 'token',
      keyringEntry: 'connectors.github.token',
      isSet: false,
      required: true,
    },
  ],
  signedIn: false,
  setupHint: 'Add a GitHub token',
} as ConnectorDetails

/** The page, re-rendered from the navigation store like MainPane does */
function Library() {
  const view = useNavigationStore(state => state.current)
  return view.kind === 'library' ? <LibraryPageView view={view} /> : null
}

function open(view: Extract<View, { kind: 'library' }>) {
  useNavigationStore.setState({ current: view, history: [] })
  return render(<Library />)
}

describe('Library', () => {
  beforeEach(() => {
    useMarketplaceStore.setState({ busy: {}, progress: {} })
    Object.assign(commands, {
      listAvailableAgents: ok([writer]),
      listMarketplacePacks: ok([research]),
      listTools: ok([shell]),
      listConnectors: ok([github]),
      listPilots: ok([]),
      listRegistries: ok([]),
      listAgentsSharedWithMe: ok([]),
      listAgentsSharedByMe: ok([]),
      installAgent: ok(null),
      grantAgentConsent: ok(null),
      setToolEnabled: ok(null),
      setConnectorSecret: ok(null),
      configureConnector: ok(null),
      startConnector: ok(null),
    })
  })

  it('shows what is installed, with a chip count per kind', async () => {
    open({ kind: 'library' })
    expect(
      await screen.findByTestId('library-card-pack:research')
    ).toBeInTheDocument()
    expect(
      screen.getByTestId('library-card-workflow:research-flow')
    ).toBeVisible()
    expect(screen.getByTestId('library-card-builtinTool:shell')).toBeVisible()
    expect(screen.getByTestId('library-card-connector:github')).toBeVisible()
    // Not installed: only on Browse
    expect(screen.queryByTestId('library-card-agent:writer')).toBeNull()
    // Needs a key before it can start
    expect(
      within(screen.getByTestId('library-card-connector:github')).getByText(
        'Needs setup'
      )
    ).toBeInTheDocument()
  })

  it('filters by kind on Browse', async () => {
    const user = userEvent.setup()
    open({ kind: 'library', tab: 'browse' })
    await screen.findByTestId('library-card-agent:writer')
    const kinds = within(screen.getByRole('group', { name: 'Kind' }))
    // Each chip carries its count
    const agents = kinds.getByRole('button', { name: /^Agents/ })
    expect(agents).toHaveTextContent('Agents1')
    await user.click(agents)
    expect(useNavigationStore.getState().current).toMatchObject({
      filter: 'agents',
    })
    expect(screen.getByTestId('library-card-agent:writer')).toBeVisible()
    expect(screen.queryByTestId('library-card-pack:research')).toBeNull()
  })

  it('opens the drawer on a card, and Back closes it', async () => {
    const user = userEvent.setup()
    open({ kind: 'library', tab: 'browse' })
    await user.click(await screen.findByTestId('library-card-agent:writer'))
    expect(useNavigationStore.getState().current).toMatchObject({
      item: 'agent:writer',
    })
    const drawer = await screen.findByRole('dialog')
    expect(within(drawer).getByText('Writer')).toBeInTheDocument()
    useNavigationStore.getState().goBack()
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull())
  })

  it('installs nothing when the consent dialog is cancelled', async () => {
    const user = userEvent.setup()
    open({ kind: 'library', tab: 'browse' })
    const card = within(await screen.findByTestId('library-card-agent:writer'))
    await user.click(card.getByRole('button', { name: 'Install' }))
    const dialog = within(await screen.findByRole('alertdialog'))
    expect(dialog.getByText('Install Writer?')).toBeInTheDocument()
    await user.click(dialog.getByRole('button', { name: 'Cancel' }))
    expect(commands.installAgent).not.toHaveBeenCalled()
  })

  it('installs and records consent for what the dialog showed', async () => {
    const consent = {
      agentId: 'writer',
      agentName: 'Writer',
      required: true,
      satisfied: false,
      categories: [
        { key: 'tools', items: ['shell', 'read_file'] },
        { key: 'network', items: ['arxiv.org'] },
        // Not in the install dialog: asked on the first run
        { key: 'folders', items: ['docs:write'] },
      ],
    } as unknown as ConsentStatus
    Object.assign(commands, { getAgentConsent: ok(consent) })
    const user = userEvent.setup()
    open({ kind: 'library', tab: 'browse' })
    const card = within(await screen.findByTestId('library-card-agent:writer'))
    await user.click(card.getByRole('button', { name: 'Install' }))
    const dialog = within(await screen.findByRole('alertdialog'))
    // Unverified publisher: the button says so
    await user.click(dialog.getByRole('button', { name: 'Install anyway' }))
    await waitFor(() =>
      expect(commands.grantAgentConsent).toHaveBeenCalledWith('writer', [
        { key: 'tools', items: ['shell', 'read_file'] },
        { key: 'network', items: ['arxiv.org'] },
      ])
    )
    expect(commands.installAgent).toHaveBeenCalledWith('writer')
  })

  it('keeps Start off until the key is saved, and saves it write-only', async () => {
    const user = userEvent.setup()
    open({ kind: 'library', item: 'connector:github' })
    const blocked = await screen.findByTestId('start-blocked')
    expect(blocked).toHaveTextContent("Can't start yet: save every key first.")
    expect(screen.getByRole('button', { name: 'Start' })).toBeDisabled()

    await user.type(screen.getByLabelText('GitHub token'), 'ghp-123')
    await user.click(screen.getByRole('button', { name: 'Save' }))
    await waitFor(() =>
      expect(commands.setConnectorSecret).toHaveBeenCalledWith(
        'github',
        'token',
        'ghp-123'
      )
    )
  })

  it('asks before clearing a saved key', async () => {
    Object.assign(commands, {
      listConnectors: ok([
        {
          ...github,
          secrets: [
            { key: 'token', keyringEntry: 'k', isSet: true, required: true },
          ],
          setupHint: null,
        },
      ]),
    })
    const user = userEvent.setup()
    open({ kind: 'library', item: 'connector:github' })
    await user.click(await screen.findByRole('button', { name: 'Clear' }))
    const dialog = within(await screen.findByRole('alertdialog'))
    expect(commands.setConnectorSecret).not.toHaveBeenCalled()
    await user.click(dialog.getByRole('button', { name: 'Remove key' }))
    await waitFor(() =>
      expect(commands.setConnectorSecret).toHaveBeenCalledWith(
        'github',
        'token',
        ''
      )
    )
  })

  it('asks before switching off a built-in tool', async () => {
    const user = userEvent.setup()
    open({ kind: 'library' })
    const card = within(
      await screen.findByTestId('library-card-builtinTool:shell')
    )
    await user.click(
      card.getByRole('switch', { name: 'Use Run shell commands' })
    )
    const dialog = within(await screen.findByRole('alertdialog'))
    expect(commands.setToolEnabled).not.toHaveBeenCalled()
    await user.click(dialog.getByRole('button', { name: 'Switch off' }))
    await waitFor(() =>
      expect(commands.setToolEnabled).toHaveBeenCalledWith('shell', false)
    )
  })

  it('offers a next step when nothing matches', async () => {
    const user = userEvent.setup()
    open({ kind: 'library', tab: 'browse', capability: 'nothing-does-this' })
    expect(
      await screen.findByText('Nothing matches these filters')
    ).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Clear filters' }))
    expect(useNavigationStore.getState().current).not.toHaveProperty(
      'capability',
      'nothing-does-this'
    )
  })
})

vi.mock('@tauri-apps/plugin-dialog', () => ({ open: vi.fn() }))
