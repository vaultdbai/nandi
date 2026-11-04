import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import type { AgentCard, AgentDelegationSettings } from '@/lib/bindings'
import { DelegationPanel } from './DelegationPanel'

const { mockClient } = vi.hoisted(() => ({
  mockClient: {
    getAgentDelegation: vi.fn(),
    setAgentDelegation: vi.fn(),
    listAvailableProviders: vi.fn(),
    listTrustPolicies: vi.fn(),
    listDelegationHistory: vi.fn(),
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

function card(
  id: string,
  capabilities: string[],
  overrides: Partial<AgentCard> = {}
): AgentCard {
  return {
    id,
    name: id.split('/')[1] ?? id,
    description: '',
    instanceId: id.split('/')[0] ?? '',
    endpoint: '',
    capabilities: capabilities.map(name => ({ name })),
    acceptsDelegation: true,
    maxConcurrent: 2,
    currentLoad: 1,
    owner: { githubHandle: '', displayName: '' },
    signature: null,
    issuedAt: 0,
    ...overrides,
  }
}

const off: AgentDelegationSettings = {
  allowOutbound: false,
  acceptInbound: false,
  maxConcurrent: null,
  overrides: [],
}

describe('DelegationPanel', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    // Saved settings are what the next read returns
    let saved = off
    mockClient.getAgentDelegation.mockImplementation(() =>
      Promise.resolve({ status: 'ok', data: saved })
    )
    mockClient.setAgentDelegation.mockImplementation(
      (_agentId: string, settings: AgentDelegationSettings) => {
        saved = settings
        return Promise.resolve({ status: 'ok', data: null })
      }
    )
    mockClient.listAvailableProviders.mockResolvedValue({
      status: 'ok',
      data: [
        card('me/writer', ['write-post']),
        card('them/papers', ['summarize'], {
          owner: { githubHandle: 'alice', displayName: 'Alice' },
        }),
        card('other/closed', ['summarize'], { acceptsDelegation: false }),
      ],
    })
    mockClient.listTrustPolicies.mockResolvedValue({
      status: 'ok',
      data: [
        {
          requester: 'them/papers',
          providerAgent: 'writer',
          policy: 'allow',
          updatedAt: 0,
        },
      ],
    })
    mockClient.listDelegationHistory.mockResolvedValue({
      status: 'ok',
      data: [],
    })
  })

  it('starts with both directions off', async () => {
    render(<DelegationPanel agentId="writer" agentName="Writer" />)
    const outbound = await screen.findByRole('switch', {
      name: 'Can call other agents',
    })
    expect(outbound).not.toBeChecked()
    expect(
      screen.getByRole('switch', { name: 'Accepts tasks from other agents' })
    ).not.toBeChecked()
    expect(screen.queryByLabelText('Tasks at once')).not.toBeInTheDocument()
  })

  it('lists who it can call by capability, and who can call it', async () => {
    render(<DelegationPanel agentId="writer" agentName="Writer" />)
    expect(await screen.findByText('@alice')).toBeInTheDocument()
    expect(screen.getByText('1 of 2 busy')).toBeInTheDocument()
    expect(screen.getByText('Not accepting')).toBeInTheDocument()
    // Its own card is left out of "can call" (listed once, in exceptions)
    expect(screen.getAllByText('write-post')).toHaveLength(1)
    expect(
      screen.getByText('Turn on “Can call other agents” to let it use these.')
    ).toBeInTheDocument()
    // Allowed by a trust rule
    await waitFor(() =>
      expect(screen.getAllByTitle('them/papers')).toHaveLength(2)
    )
  })

  it('saves toggles into the agent’s settings', async () => {
    const user = userEvent.setup()
    render(<DelegationPanel agentId="writer" agentName="Writer" />)
    await user.click(
      await screen.findByRole('switch', { name: 'Can call other agents' })
    )
    expect(mockClient.setAgentDelegation).toHaveBeenCalledWith('writer', {
      ...off,
      allowOutbound: true,
    })
    // Letting other Nandis send work asks once first
    mockClient.setAgentDelegation.mockClear()
    await user.click(
      screen.getByRole('switch', { name: 'Accepts tasks from other agents' })
    )
    expect(mockClient.setAgentDelegation).not.toHaveBeenCalled()
    await user.click(await screen.findByRole('button', { name: 'Turn on' }))
    expect(mockClient.setAgentDelegation).toHaveBeenLastCalledWith('writer', {
      ...off,
      allowOutbound: true,
      acceptInbound: true,
    })
  })

  it('edits per-capability exceptions', async () => {
    const user = userEvent.setup()
    render(<DelegationPanel agentId="writer" agentName="Writer" />)
    // The agent's own capabilities are listed, deferring to the setting
    const policy = await screen.findByLabelText('Policy for write-post')
    expect(policy).toHaveValue('ask')
    await user.selectOptions(policy, 'deny')
    expect(mockClient.setAgentDelegation).toHaveBeenLastCalledWith('writer', {
      ...off,
      overrides: [{ capability: 'write-post', policy: 'deny' }],
    })

    await user.type(screen.getByLabelText('capability-name'), 'translate')
    await user.click(screen.getByRole('button', { name: 'Add' }))
    await waitFor(() =>
      expect(mockClient.setAgentDelegation).toHaveBeenLastCalledWith(
        'writer',
        expect.objectContaining({
          overrides: expect.arrayContaining([
            { capability: 'translate', policy: 'allow' },
          ]),
        })
      )
    )
  })

  it('says why it can’t list agents to call when delegation is unavailable', async () => {
    mockClient.listAvailableProviders.mockResolvedValue({
      status: 'error',
      error: 'Delegation is unavailable',
    })
    render(<DelegationPanel agentId="writer" agentName="Writer" />)
    expect(
      await screen.findByText(
        'Can’t list agents to call: Delegation is unavailable'
      )
    ).toBeInTheDocument()
    expect(
      screen.queryByText('No other agent serves a capability yet.')
    ).not.toBeInTheDocument()
  })

  it('shows a load error', async () => {
    mockClient.getAgentDelegation.mockImplementation(() =>
      Promise.resolve({ status: 'error', error: 'Agent not found' })
    )
    render(<DelegationPanel agentId="ghost" agentName="Ghost" />)
    expect(await screen.findByRole('alert')).toHaveTextContent(
      'Agent not found'
    )
  })
})
