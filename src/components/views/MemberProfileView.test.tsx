import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { toast } from 'sonner'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { agentCard, ok } from '@/hooks/useSharedAgents.fixtures'
import type { MemberProfile, RelayStatus, TaskRecord } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { MemberProfileView } from './MemberProfileView'

vi.mock('sonner', () => ({
  toast: { success: vi.fn(), error: vi.fn(), warning: vi.fn() },
}))

const trust = {
  instanceId: 'i-alice',
  addedAsTrustedPeer: false,
  delegationsPaused: false,
  blockedAgents: [],
  lastUpdated: null,
}

const task: TaskRecord = {
  id: 'task-1',
  requesterInstance: 'me',
  requesterAgent: '_user',
  providerInstance: 'i-alice',
  providerAgent: 'papers',
  capability: 'find-papers',
  input: null,
  state: 'completed',
  etaSeconds: null,
  deadline: null,
  priority: 0,
  createdAt: Date.now() - 60_000,
  updatedAt: Date.now(),
  acknowledgedAt: null,
  completedAt: Date.now(),
  error: null,
  result: null,
  parentTaskId: null,
  sourceKind: 'direct',
  attemptNumber: 1,
  previousAttemptId: null,
}

const profile: MemberProfile = {
  instanceId: 'i-alice',
  handle: 'alice',
  displayName: 'Alice',
  avatarUrl: null,
  online: true,
  lastSeen: Date.now(),
  connected: true,
  gateway: {
    instanceId: 'i-alice',
    endpoint: '',
    displayName: 'Alice',
    description: 'Research help',
    capabilities: ['find-papers'],
    acceptsDelegation: true,
    maxConcurrent: 3,
    currentLoad: 1,
    owner: { githubHandle: 'alice', displayName: 'Alice' },
    federationEnabled: false,
    gatewayVersion: 1,
    agentCount: 2,
    body: null,
  },
  capabilities: ['find-papers'],
  agents: [
    {
      card: agentCard('i-alice', 'papers', { name: 'Papers' }),
      source: 'team',
      teamId: 'acme',
      blocked: false,
    },
    {
      card: agentCard('i-alice', 'notes', { name: 'Notes' }),
      source: 'direct',
      teamId: null,
      blocked: false,
    },
  ],
  delegationStats: {
    sent: 3,
    received: 1,
    succeeded: 3,
    failed: 1,
    avgDurationMs: 2000,
    lastAt: Date.now(),
  },
  recentDelegations: [task],
  trust,
}

const relay = (connected: boolean): RelayStatus => ({
  connected,
  state: connected ? 'connected' : 'disconnected',
  reason: null,
  instanceId: 'me',
  relayUrl: connected ? 'wss://relay.example' : null,
  relayUrlSource: connected ? 'config' : null,
  builtInUrl: null,
  agentsOnline: 0,
  instancesOnline: 0,
  lastConnectedAt: null,
  lastHeartbeatAt: null,
})

describe('MemberProfileView', () => {
  beforeEach(() => {
    Object.assign(commands, {
      getRelayStatus: ok(relay(true)),
      getMemberByInstance: ok(profile),
      blockAgentFromMember: ok({ ...trust, blockedAgents: ['notes'] }),
      unblockAgentFromMember: ok(trust),
      pauseDelegationsFromMember: ok({ ...trust, delegationsPaused: true }),
      setTrustRelationship: ok({ ...trust, addedAsTrustedPeer: true }),
      delegateTask: ok('task-new'),
    })
    useNavigationStore.setState({
      current: { kind: 'member-profile', instanceId: 'i-alice' },
      history: [],
    })
    vi.mocked(toast.error).mockClear()
  })

  describe('Ask', () => {
    it("sends a plain-words request to the member's gateway and opens its thread", async () => {
      const user = userEvent.setup()
      render(<MemberProfileView instanceId="i-alice" />)
      const ask = within(await screen.findByTestId('ask-member'))
      expect(
        ask.getByRole('heading', { name: 'Ask Alice' })
      ).toBeInTheDocument()
      const send = ask.getByRole('button', { name: 'Send' })
      expect(send).toBeDisabled()

      await user.type(
        ask.getByLabelText('Ask Alice'),
        '  Find recent papers on CRDTs '
      )
      await user.click(send)

      await waitFor(() =>
        expect(commands.delegateTask).toHaveBeenCalledWith(
          'gateway',
          { text: 'Find recent papers on CRDTs' },
          'i-alice/gateway'
        )
      )
      await waitFor(() =>
        expect(useNavigationStore.getState().current).toEqual({
          kind: 'delegation-thread',
          taskId: 'task-new',
        })
      )
    })

    it('offers the gateway capabilities as examples', async () => {
      const user = userEvent.setup()
      render(<MemberProfileView instanceId="i-alice" />)
      const ask = within(await screen.findByTestId('ask-member'))
      await user.click(ask.getByRole('button', { name: 'find-papers' }))
      expect(ask.getByLabelText('Ask Alice')).toHaveValue(
        'Can you help with find-papers? '
      )
      expect(ask.getByRole('button', { name: 'Send' })).toBeEnabled()
    })

    it('toasts when sending fails and stays put', async () => {
      const user = userEvent.setup()
      Object.assign(commands, {
        delegateTask: vi
          .fn()
          .mockResolvedValue({ status: 'error', error: 'relay down' }),
      })
      render(<MemberProfileView instanceId="i-alice" />)
      const ask = within(await screen.findByTestId('ask-member'))
      await user.type(ask.getByLabelText('Ask Alice'), 'hello')
      await user.click(ask.getByRole('button', { name: 'Send' }))

      await waitFor(() =>
        expect(toast.error).toHaveBeenCalledWith(
          "Couldn't send the request: relay down"
        )
      )
      expect(useNavigationStore.getState().current.kind).toBe('member-profile')
      expect(ask.getByLabelText('Ask Alice')).toHaveValue('hello')
    })

    it('shows "Connect to your company’s relay" when sending answers not connected', async () => {
      const user = userEvent.setup()
      Object.assign(commands, {
        delegateTask: vi.fn().mockResolvedValue({
          status: 'error',
          error:
            "relay_unavailable: this needs your company's relay, and Nandi isn't connected to one",
        }),
      })
      render(<MemberProfileView instanceId="i-alice" />)
      const ask = within(await screen.findByTestId('ask-member'))
      await user.type(ask.getByLabelText('Ask Alice'), 'hello')
      await user.click(ask.getByRole('button', { name: 'Send' }))

      expect(await ask.findByTestId('relay-connect-notice')).toHaveTextContent(
        'Connect to your company’s relay'
      )
      expect(toast.error).not.toHaveBeenCalled()
      expect(ask.getByRole('button', { name: 'Send' })).toBeDisabled()
    })

    it('asks to connect to the relay, not to send, while disconnected', async () => {
      Object.assign(commands, {
        getRelayStatus: ok(relay(false)),
      })
      render(<MemberProfileView instanceId="i-alice" />)
      const ask = within(await screen.findByTestId('ask-member'))
      expect(await ask.findByTestId('relay-connect-notice')).toBeInTheDocument()
      expect(ask.getByLabelText('Ask Alice')).toBeDisabled()
      expect(ask.queryByTestId('ask-unavailable')).toBeNull()
    })

    it('says why a member offline can’t be asked', async () => {
      Object.assign(commands, {
        getMemberByInstance: ok({ ...profile, online: false }),
      })
      render(<MemberProfileView instanceId="i-alice" />)
      const ask = within(await screen.findByTestId('ask-member'))
      expect(ask.getByTestId('ask-unavailable')).toHaveTextContent(
        'Alice is offline'
      )
      expect(ask.getByLabelText('Ask Alice')).toBeDisabled()
      expect(ask.getByRole('button', { name: 'Send' })).toBeDisabled()
    })

    it('says when their gateway isn’t accepting requests', async () => {
      const gateway = profile.gateway
      if (!gateway) throw new Error('fixture has a gateway')
      Object.assign(commands, {
        getMemberByInstance: ok({
          ...profile,
          gateway: { ...gateway, acceptsDelegation: false },
        }),
      })
      render(<MemberProfileView instanceId="i-alice" />)
      const ask = within(await screen.findByTestId('ask-member'))
      expect(ask.getByTestId('ask-unavailable')).toHaveTextContent(
        "Alice's Nandi isn't accepting requests right now."
      )
      expect(ask.getByRole('button', { name: 'Send' })).toBeDisabled()
    })
  })

  it('shows the gateway, then agents by source in the capabilities tab', async () => {
    const user = userEvent.setup()
    render(<MemberProfileView instanceId="i-alice" />)
    expect(await screen.findByText('Research help')).toBeInTheDocument()
    await user.click(screen.getByRole('tab', { name: 'Capabilities' }))
    expect(
      screen.getByTestId('member-agent-i-alice/papers')
    ).toBeInTheDocument()
    expect(screen.getByText('Shared to a team')).toBeInTheDocument()
    expect(screen.getByText('Direct')).toBeInTheDocument()
    expect(screen.getAllByText('find-papers').length).toBeGreaterThan(0)
  })

  it('shows what the member is responsible for, even without their NANDI.md', async () => {
    const gateway = profile.gateway
    if (!gateway) throw new Error('fixture has a gateway')
    Object.assign(commands, {
      getMemberByInstance: ok({
        ...profile,
        gateway: { ...gateway, responsibilities: ['SOX audit', 'Risk'] },
      }),
    })
    render(<MemberProfileView instanceId="i-alice" />)
    const list = within(await screen.findByTestId('member-responsibilities'))
    expect(list.getByText('Responsible for')).toBeInTheDocument()
    expect(list.getByText('SOX audit')).toBeInTheDocument()
    expect(list.getByText('Risk')).toBeInTheDocument()
    expect(
      screen.getByText("This member doesn't share their NANDI.md.")
    ).toBeInTheDocument()
  })

  it('lists no responsibilities when the card has none', async () => {
    render(<MemberProfileView instanceId="i-alice" />)
    await screen.findByText('Research help')
    expect(screen.queryByTestId('member-responsibilities')).toBeNull()
  })

  it('opens a delegation thread from the recent delegations tab', async () => {
    const user = userEvent.setup()
    render(<MemberProfileView instanceId="i-alice" />)
    await user.click(
      await screen.findByRole('tab', { name: 'Recent delegations' })
    )
    await user.click(await screen.findByTestId('member-task-task-1'))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'delegation-thread',
      taskId: 'task-1',
    })
  })

  it('blocks the whole member from the trust tab', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      blockAgentFromMember: ok({ ...trust, blockedAgents: ['*'] }),
    })
    render(<MemberProfileView instanceId="i-alice" />)
    await user.click(await screen.findByRole('tab', { name: 'Trust' }))
    await user.click(screen.getByRole('switch', { name: /block this member/i }))
    await waitFor(() =>
      expect(commands.blockAgentFromMember).toHaveBeenCalledWith('i-alice', '*')
    )
  })

  it('blocks an agent and pauses delegations', async () => {
    const user = userEvent.setup()
    render(<MemberProfileView instanceId="i-alice" />)
    await user.click(await screen.findByRole('tab', { name: 'Capabilities' }))
    const notes = within(
      await screen.findByTestId('member-agent-i-alice/notes')
    )
    await user.click(notes.getByRole('button', { name: /block/i }))
    await waitFor(() =>
      expect(commands.blockAgentFromMember).toHaveBeenCalledWith(
        'i-alice',
        'notes'
      )
    )
    await user.click(screen.getByRole('tab', { name: 'Trust' }))
    await user.click(screen.getByRole('switch', { name: /pause delegations/i }))
    await waitFor(() =>
      expect(commands.pauseDelegationsFromMember).toHaveBeenCalledWith(
        'i-alice',
        true
      )
    )
    await user.click(screen.getByRole('switch', { name: /trusted peer/i }))
    await waitFor(() =>
      expect(commands.setTrustRelationship).toHaveBeenCalledWith('i-alice', {
        addedAsTrustedPeer: true,
        delegationsPaused: null,
      })
    )
  })

  it("says when a member hasn't connected their Nandi yet", async () => {
    Object.assign(commands, {
      getMemberByInstance: ok({
        ...profile,
        connected: false,
        online: false,
        gateway: null,
        agents: [],
        recentDelegations: [],
      }),
    })
    render(<MemberProfileView instanceId="i-alice" />)
    expect(
      (
        await screen.findAllByText(
          "This member hasn't connected their Nandi yet"
        )
      ).length
    ).toBeGreaterThan(0)
    expect(screen.queryByTestId('member-gateway')).toBeNull()
    expect(screen.queryByTestId('ask-member')).toBeNull()
  })
})
