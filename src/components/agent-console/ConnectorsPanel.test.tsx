import { describe, it, expect, beforeEach, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import type { ConnectorDetails } from '@/hooks/useConnectors'
import { ConnectorsPanel } from './ConnectorsPanel'

const { mockClient } = vi.hoisted(() => ({
  mockClient: {
    listConnectors: vi.fn(),
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

const base = {
  description: '',
  kind: 'mcp',
  config: {},
  secrets: [],
  oauthFlow: null,
  signedIn: false,
  setupHint: null,
}

const connectors: ConnectorDetails[] = [
  {
    ...base,
    id: 'filesystem',
    name: 'Filesystem',
    status: { installed: true, running: true, healthy: true, error: null },
    tools: [
      { name: 'read_file', description: '', sensitive: false },
      { name: 'write_file', description: '', sensitive: true },
    ],
  },
  {
    ...base,
    id: 'web_search',
    name: 'Web search',
    status: { installed: true, running: false, healthy: false, error: null },
    tools: [{ name: 'search', description: '', sensitive: false }],
  },
  {
    ...base,
    id: 'gmail',
    name: 'Gmail',
    status: { installed: false, running: false, healthy: false, error: null },
    tools: [],
  },
]

describe('ConnectorsPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mockClient.listConnectors.mockResolvedValue({
      status: 'ok',
      data: connectors,
    })
  })

  it('lists installed connectors with the tools they expose', async () => {
    render(<ConnectorsPanel agentId="writer" enabled={['filesystem']} />)
    expect(await screen.findByText('Filesystem')).toBeInTheDocument()
    expect(screen.getByText('Web search')).toBeInTheDocument()
    expect(screen.queryByText('Gmail')).not.toBeInTheDocument()
    expect(screen.getByText('filesystem__write_file')).toBeInTheDocument()
    expect(screen.getByText('web_search__search')).toBeInTheDocument()
    const toggles = screen.getAllByRole('switch')
    expect(toggles[0]).toBeChecked()
    expect(toggles[1]).not.toBeChecked()
    // No owner to save the change: read-only
    expect(toggles[0]).toBeDisabled()
  })

  it('reports toggles to the owner', async () => {
    const user = userEvent.setup()
    const onEnabledChange = vi.fn()
    render(
      <ConnectorsPanel
        agentId="writer"
        enabled={['filesystem']}
        onEnabledChange={onEnabledChange}
      />
    )
    await screen.findByText('Filesystem')
    const toggles = screen.getAllByRole('switch')
    await user.click(toggles[1] as HTMLElement)
    expect(onEnabledChange).toHaveBeenLastCalledWith([
      'filesystem',
      'web_search',
    ])
    await user.click(toggles[0] as HTMLElement)
    expect(onEnabledChange).toHaveBeenLastCalledWith([])
  })

  it('says when nothing is installed', async () => {
    mockClient.listConnectors.mockResolvedValue({ status: 'ok', data: [] })
    render(<ConnectorsPanel />)
    expect(
      await screen.findByText(
        'No connectors installed. Install them from Library › Browse › Tools.'
      )
    ).toBeInTheDocument()
  })
})
