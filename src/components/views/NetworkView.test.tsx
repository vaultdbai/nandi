import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { ok } from '@/hooks/useSharedAgents.fixtures'
import { filterMembers, initialsOf, searchMembers } from '@/hooks/useNetwork'
import type {
  GatewayStatus,
  NetworkMember,
  NetworkSnapshot,
  RelayStatus,
} from '@/lib/bindings'
import { MANAGE_TEAMS, useNavigationStore } from '@/store/navigation-store'
import { NetworkView } from './NetworkView'

const HOUR = 60 * 60 * 1000

function member(
  instanceId: string,
  displayName: string,
  overrides: Partial<NetworkMember> = {}
): NetworkMember {
  return {
    instanceId,
    displayName,
    avatarUrl: null,
    online: true,
    lastSeen: Date.now(),
    handle: displayName.toLowerCase(),
    gateway: {
      instanceId,
      endpoint: '',
      displayName,
      description: `${displayName}'s Nandi`,
      capabilities: ['find-papers', 'summarize'],
      acceptsDelegation: true,
      maxConcurrent: 3,
      currentLoad: 0,
      owner: { githubHandle: displayName.toLowerCase(), displayName },
      federationEnabled: false,
      gatewayVersion: 1,
      agentCount: 4,
      body: null,
    },
    ...overrides,
  }
}

const snapshot = (
  members: NetworkMember[],
  relayConnected = true
): NetworkSnapshot => ({
  relayUrl: 'wss://relay.example',
  relayConnected,
  members,
  fetchedAt: Date.now(),
})

const gatewayStatus = (reachable: boolean): GatewayStatus => ({
  enabled: reachable,
  registered: reachable,
  discoverable: reachable,
  card: null,
  activeTasks: 0,
  queuedTasks: 0,
  lastHeartbeatAt: null,
})

const relayStatus = (connected: boolean): RelayStatus => ({
  connected,
  state: connected ? 'connected' : 'disconnected',
  reason: null,
  instanceId: 'me',
  relayUrl: 'wss://relay.example',
  relayUrlSource: 'config',
  agentsOnline: 0,
  instancesOnline: 0,
  lastConnectedAt: null,
  lastHeartbeatAt: null,
})

const alice = member('i-alice', 'Alice')
const bob = member('i-bob', 'Bob', {
  online: false,
  lastSeen: Date.now() - 2 * HOUR,
})
const carol = member('i-carol', 'Carol', {
  online: false,
  lastSeen: Date.now() - 72 * HOUR,
})

describe('NetworkView', () => {
  beforeEach(() => {
    useNavigationStore.setState({ current: { kind: 'network' }, history: [] })
    Object.assign(commands, {
      getNetworkSnapshot: ok(snapshot([alice, bob, carol])),
      refreshNetwork: ok(snapshot([alice])),
      delegateTask: ok('task-ask'),
      getGatewayStatus: ok(gatewayStatus(true)),
      getRelayStatus: ok(relayStatus(true)),
      listOnlineAgents: ok([]),
      listTeams: ok([]),
    })
  })

  it('offers Make Reachable while the gateway is off or unlisted', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getGatewayStatus: ok(gatewayStatus(false)),
      makeGatewayReachable: ok(gatewayStatus(true)),
    })
    render(<NetworkView />)
    const banner = await screen.findByTestId('network-not-reachable')
    expect(banner).toHaveTextContent(
      /not visible to the network|network\.reachable\.title/i
    )
    await user.click(
      within(banner).getByRole('button', {
        name: /make reachable|network\.reachable\.action/i,
      })
    )
    await waitFor(() =>
      expect(commands.makeGatewayReachable).toHaveBeenCalledTimes(1)
    )
  })

  it('shows no reachability banner while disconnected', async () => {
    Object.assign(commands, {
      getGatewayStatus: ok(gatewayStatus(false)),
      getRelayStatus: ok(relayStatus(false)),
    })
    render(<NetworkView />)
    await screen.findByTestId('member-i-alice')
    expect(screen.queryByTestId('network-not-reachable')).toBeNull()
  })

  it('shows no banner for a reachable Nandi', async () => {
    render(<NetworkView />)
    await screen.findByTestId('member-i-alice')
    expect(screen.queryByTestId('network-not-reachable')).toBeNull()
  })

  it('shows each member with presence, capabilities, and agents', async () => {
    render(<NetworkView />)
    const card = within(await screen.findByTestId('member-i-alice'))
    expect(card.getByText('Alice')).toBeInTheDocument()
    expect(card.getByText('@alice')).toBeInTheDocument()
    expect(card.getByText("Alice's Nandi")).toBeInTheDocument()
    expect(card.getByText(/Capabilities: 2/)).toBeInTheDocument()
    expect(card.getByText(/Agents: 4/)).toBeInTheDocument()
    expect(screen.getByTestId('member-i-bob')).toBeInTheDocument()
  })

  it('opens the member profile from a card', async () => {
    const user = userEvent.setup()
    render(<NetworkView />)
    const card = within(await screen.findByTestId('member-i-alice'))
    await user.click(card.getByRole('button', { name: /open profile/i }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'member-profile',
      instanceId: 'i-alice',
    })
  })

  it("asks a member's gateway from their card", async () => {
    const user = userEvent.setup()
    render(<NetworkView />)
    const card = within(await screen.findByTestId('member-i-alice'))
    await user.click(card.getByRole('button', { name: /^ask$/i }))

    const ask = within(await screen.findByTestId('ask-member'))
    await user.type(ask.getByLabelText('Ask Alice'), 'Summarize my notes')
    await user.click(ask.getByRole('button', { name: 'Send' }))

    await waitFor(() =>
      expect(commands.delegateTask).toHaveBeenCalledWith(
        'gateway',
        { text: 'Summarize my notes' },
        'i-alice/gateway'
      )
    )
    await waitFor(() =>
      expect(useNavigationStore.getState().current).toEqual({
        kind: 'delegation-thread',
        taskId: 'task-ask',
      })
    )
  })

  it("can't ask an offline member or one not accepting requests", async () => {
    Object.assign(commands, {
      getNetworkSnapshot: ok(
        snapshot([
          bob,
          member('i-dan', 'Dan', {
            gateway: {
              ...alice.gateway,
              instanceId: 'i-dan',
              acceptsDelegation: false,
            },
          }),
        ])
      ),
    })
    render(<NetworkView />)
    const bobCard = within(await screen.findByTestId('member-i-bob'))
    const bobAsk = bobCard.getByRole('button', { name: /^ask$/i })
    expect(bobAsk).toBeDisabled()
    expect(bobAsk).toHaveAttribute('title', expect.stringContaining('offline'))
    const danAsk = within(screen.getByTestId('member-i-dan')).getByRole(
      'button',
      { name: /^ask$/i }
    )
    expect(danAsk).toBeDisabled()
    expect(danAsk).toHaveAttribute(
      'title',
      "Dan's Nandi isn't accepting requests right now."
    )
  })

  it('filters online and recently active members', async () => {
    const user = userEvent.setup()
    render(<NetworkView />)
    await screen.findByTestId('member-i-carol')
    await user.click(screen.getByRole('radio', { name: /^online$/i }))
    expect(screen.getByTestId('member-i-alice')).toBeInTheDocument()
    expect(screen.queryByTestId('member-i-bob')).toBeNull()
    await user.click(screen.getByRole('radio', { name: /recently active/i }))
    expect(screen.getByTestId('member-i-bob')).toBeInTheDocument()
    expect(screen.queryByTestId('member-i-carol')).toBeNull()
  })

  it('refreshes from the relay', async () => {
    const user = userEvent.setup()
    render(<NetworkView />)
    await screen.findByTestId('member-i-carol')
    await user.click(screen.getByRole('button', { name: /refresh/i }))
    expect(commands.refreshNetwork).toHaveBeenCalled()
    expect(await screen.findByTestId('member-i-alice')).toBeInTheDocument()
    expect(screen.queryByTestId('member-i-carol')).toBeNull()
  })

  it('asks calmly to connect to the relay while disconnected', async () => {
    Object.assign(commands, {
      getNetworkSnapshot: ok(snapshot([], false)),
      getRelayStatus: ok(relayStatus(false)),
    })
    const user = userEvent.setup()
    render(<NetworkView />)
    const notice = await screen.findByTestId('relay-connect-notice')
    expect(
      within(notice).getByText('Connect to your company’s relay')
    ).toBeInTheDocument()
    await user.click(
      within(notice).getByRole('button', { name: 'Open connection settings' })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'network',
      tab: 'connection',
    })
  })

  it('shows the last listing, marked as last seen, while disconnected', async () => {
    Object.assign(commands, {
      getNetworkSnapshot: ok(snapshot([{ ...bob }, { ...carol }], false)),
      getRelayStatus: ok(relayStatus(false)),
    })
    render(<NetworkView />)
    expect(await screen.findByTestId('member-i-bob')).toBeInTheDocument()
    expect(screen.getByTestId('network-last-seen')).toHaveTextContent(
      'Showing everyone as last seen.'
    )
    expect(screen.queryByTestId('relay-connect-notice')).toBeNull()
  })

  it('says when someone is away and who covers', async () => {
    const lena = member('i-lena', 'Lena')
    const priya = member('i-priya', 'Priya')
    priya.gateway.availability = {
      status: 'away',
      until: null,
      delegateTo: 'i-lena',
    }
    Object.assign(commands, {
      getNetworkSnapshot: ok(snapshot([lena, priya])),
    })
    render(<NetworkView />)
    const card = await screen.findByTestId('member-i-priya')
    await waitFor(() =>
      expect(within(card).getByTestId('member-availability')).toHaveTextContent(
        'Away · Lena covers'
      )
    )
    expect(
      within(screen.getByTestId('member-i-lena')).queryByTestId(
        'member-availability'
      )
    ).toBeNull()
  })

  it('says "You cover" when this Nandi covers (it is not in the list)', async () => {
    const priya = member('i-priya', 'Priya')
    priya.gateway.availability = {
      status: 'away',
      until: null,
      delegateTo: 'me',
    }
    Object.assign(commands, {
      getNetworkSnapshot: ok(snapshot([priya])),
    })
    render(<NetworkView />)
    const card = await screen.findByTestId('member-i-priya')
    await waitFor(() =>
      expect(within(card).getByTestId('member-availability')).toHaveTextContent(
        'Away · You cover'
      )
    )
  })

  it('invites someone when this is the only Nandi on the relay', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    })
    Object.assign(commands, {
      getNetworkSnapshot: ok(snapshot([])),
      getRelayStatus: ok({
        ...relayStatus(true),
        relayUrl: 'wss://relay.example/relay?token=s3cret',
      }),
      getRelayInvite: ok({
        url: 'wss://relay.example/relay',
        joinCode: 'acme 2026',
      }),
    })
    render(<NetworkView />)
    expect(
      await screen.findByText("You're the only Nandi on this relay.")
    ).toBeInTheDocument()
    await userEvent.click(
      screen.getByRole('button', { name: 'Copy invite message' })
    )
    // The address and the join code a teammate pastes, never the token
    await waitFor(() =>
      expect(writeText).toHaveBeenCalledWith(
        expect.stringContaining('wss://relay.example/relay#join=acme%202026')
      )
    )
    expect(writeText).not.toHaveBeenCalledWith(
      expect.stringContaining('s3cret')
    )
    // Nothing on screen shows the code
    expect(document.body).not.toHaveTextContent('acme 2026')
  })

  it('never shows the device ID of a member without a handle', async () => {
    Object.assign(commands, {
      getNetworkSnapshot: ok(
        snapshot([member('i-anon-1234', 'Anon', { handle: null })])
      ),
    })
    render(<NetworkView />)
    const card = await screen.findByTestId('member-i-anon-1234')
    expect(card).not.toHaveTextContent('i-anon-1234')
    expect(within(card).getByText('Unnamed Nandi')).toBeInTheDocument()
    expect(
      within(card).getByRole('button', { name: 'Copy ID' })
    ).toBeInTheDocument()
  })

  it('opens Manage teams from the team filter', async () => {
    const user = userEvent.setup()
    render(<NetworkView />)
    await screen.findByTestId('member-i-alice')
    await user.selectOptions(
      screen.getByRole('combobox', { name: 'Team' }),
      'Manage teams…'
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'network',
      tab: 'people',
      team: MANAGE_TEAMS,
    })
  })
})

describe('network helpers', () => {
  it('filters by presence and last seen', () => {
    const now = Date.now()
    const members = [alice, bob, carol]
    expect(filterMembers(members, 'all', now)).toHaveLength(3)
    expect(filterMembers(members, 'online', now)).toEqual([alice])
    expect(filterMembers(members, 'recent', now)).toEqual([alice, bob])
  })

  it('searches names, handles and what members can do', () => {
    expect(searchMembers([alice, bob], 'summar')).toEqual([alice, bob])
    expect(searchMembers([alice, bob], 'bob')).toEqual([bob])
    expect(searchMembers([alice, bob], '  ')).toEqual([alice, bob])
  })

  it("searches what members say they're responsible for", () => {
    const owner = member('i-dana', 'Dana')
    const dana = {
      ...owner,
      gateway: { ...owner.gateway, responsibilities: ['SOX audit', 'Risk'] },
    }
    expect(searchMembers([alice, dana], 'sox')).toEqual([dana])
    // Cards from older Nandis have none
    expect(searchMembers([alice], 'sox')).toEqual([])
  })

  it('makes initials', () => {
    expect(initialsOf('Alice Smith')).toBe('AS')
    expect(initialsOf('bob')).toBe('BO')
    expect(initialsOf('  ')).toBe('?')
  })
})
