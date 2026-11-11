import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { ok } from '@/hooks/useSharedAgents.fixtures'
import {
  gatewayStatusFixture,
  manifestFixture,
  relayStatusFixture,
} from '@/components/network/network-fixtures'
import { useNavigationStore } from '@/store/navigation-store'
import { ConnectionView } from './ConnectionView'

describe('ConnectionView', () => {
  it('says to update Nandi or the relay when their versions differ', async () => {
    Object.assign(commands, {
      getRelayStatus: ok(
        relayStatusFixture({
          connected: false,
          state: 'error',
          versionMismatch: true,
          reason: 'The relay refused us: version',
        })
      ),
      getGatewayStatus: ok(gatewayStatusFixture()),
      getGatewayManifest: ok(manifestFixture()),
      getRelayInstances: ok([]),
      listOnlineAgents: ok([]),
      listCallerPolicies: ok([]),
    })
    render(<ConnectionView />)
    expect(await screen.findByTestId('relay-error')).toHaveTextContent(
      'Update Nandi or the relay'
    )
  })

  it('keeps the usual words for other refusals', async () => {
    Object.assign(commands, {
      getRelayStatus: ok(
        relayStatusFixture({ connected: false, state: 'error' })
      ),
      getGatewayStatus: ok(gatewayStatusFixture()),
      getGatewayManifest: ok(manifestFixture()),
      getRelayInstances: ok([]),
      listOnlineAgents: ok([]),
      listCallerPolicies: ok([]),
    })
    render(<ConnectionView />)
    expect(await screen.findByTestId('relay-error')).toHaveTextContent(
      "Can't reach the relay"
    )
  })

  it("shows the relay's address without this Nandi's token", async () => {
    Object.assign(commands, {
      getRelayStatus: ok(
        relayStatusFixture({
          relayUrl: 'wss://relay.example.com/relay?token=s3cret',
        })
      ),
      getGatewayStatus: ok(gatewayStatusFixture()),
      getGatewayManifest: ok(manifestFixture()),
      getRelayInstances: ok([]),
      listOnlineAgents: ok([]),
      listCallerPolicies: ok([]),
    })
    render(<ConnectionView />)
    const url = await screen.findByTestId('relay-url')
    expect(url).toHaveTextContent('wss://relay.example.com/relay')
    expect(url).not.toHaveTextContent('s3cret')
  })

  const disconnected = (overrides = {}) =>
    Object.assign(commands, {
      getRelayStatus: ok(
        relayStatusFixture({
          connected: false,
          state: 'disconnected',
          relayUrl: null,
          relayUrlSource: null,
        })
      ),
      getGatewayStatus: ok(gatewayStatusFixture()),
      getGatewayManifest: ok(manifestFixture()),
      getRelayInstances: ok([]),
      listOnlineAgents: ok([]),
      listCallerPolicies: ok([]),
      joinRelay: ok({ kind: 'joined', relay: 'wss://relay.acme.test/relay' }),
      ...overrides,
    })

  it('never prefills a developer address; a build relay prefills itself', async () => {
    disconnected()
    render(<ConnectionView />)
    const url = await screen.findByLabelText('Relay URL')
    expect(url).toHaveValue('')
    expect(url).toHaveAttribute('placeholder', 'wss://relay.example.com/relay')
    expect(document.body).not.toHaveTextContent('127.0.0.1:8787')
  })

  it("prefills the build's own relay", async () => {
    disconnected({
      getRelayStatus: ok(
        relayStatusFixture({
          connected: false,
          state: 'disconnected',
          relayUrl: null,
          relayUrlSource: null,
          builtInUrl: 'wss://relay.acme.test/relay',
        })
      ),
    })
    render(<ConnectionView />)
    expect(await screen.findByLabelText('Relay URL')).toHaveValue(
      'wss://relay.acme.test/relay'
    )
  })

  it('shows trust off, with the reason, where it cannot apply', async () => {
    // Saved as on, but no relay yet
    disconnected({
      getRelayTrust: ok({ enabled: true, secured: false, public: false }),
    })
    const { unmount } = render(<ConnectionView />)
    const card = await screen.findByTestId('relay-trust')
    const toggle = await screen.findByRole('switch')
    expect(toggle).not.toBeChecked()
    expect(toggle).toBeDisabled()
    expect(card).toHaveTextContent("Applies once you join your company's relay")
    unmount()

    // On the public relay: off, and it says why
    disconnected({
      getRelayStatus: ok(
        relayStatusFixture({ relayUrl: 'wss://relay.vaultdb.ai/relay' })
      ),
      getRelayTrust: ok({ enabled: true, secured: false, public: true }),
      getPublicRelayAccount: ok({ kind: 'notPublic' }),
    })
    render(<ConnectionView />)
    const publicCard = await screen.findByTestId('relay-trust')
    await waitFor(() =>
      expect(publicCard).toHaveTextContent(
        'Anyone with an email can join the public relay'
      )
    )
    expect(within(publicCard).getByRole('switch')).not.toBeChecked()
  })

  it('joins a company relay with its address and join code', async () => {
    disconnected()
    const user = userEvent.setup()
    render(<ConnectionView />)
    const url = await screen.findByLabelText('Relay URL')
    await user.clear(url)
    await user.type(url, 'wss://relay.acme.test/relay')
    const code = screen.getByLabelText('Join code')
    expect(code).toHaveAttribute('type', 'password')
    await user.type(code, 'acme-2026')
    await user.click(screen.getByRole('button', { name: 'Join' }))
    await waitFor(() =>
      expect(commands.joinRelay).toHaveBeenCalledWith(
        'wss://relay.acme.test/relay',
        'acme-2026'
      )
    )
  })

  it("shows a full relay's own words, and never the join code", async () => {
    disconnected({
      joinRelay: ok({
        kind: 'relayFull',
        message:
          'This relay is at its free limit of 20 users. Add a licence file from VaultDB.ai to add more.',
      }),
    })
    const user = userEvent.setup()
    render(<ConnectionView />)
    const url = await screen.findByLabelText('Relay URL')
    await user.clear(url)
    await user.type(url, 'wss://relay.acme.test/relay')
    await user.type(screen.getByLabelText('Join code'), 'acme-2026')
    await user.click(screen.getByRole('button', { name: 'Join' }))
    const result = await screen.findByTestId('relay-join-result')
    expect(result).toHaveTextContent('relay.acme.test has no free seat')
    expect(result).toHaveTextContent(
      'This relay is at its free limit of 20 users. Add a licence file from VaultDB.ai to add more.'
    )
    expect(document.body).not.toHaveTextContent('acme-2026')
  })

  it('fills both fields from an invite on the clipboard', async () => {
    disconnected()
    // After setup: user-event puts its own clipboard in place
    const user = userEvent.setup()
    Object.defineProperty(navigator, 'clipboard', {
      value: {
        readText: vi
          .fn()
          .mockResolvedValue(
            'Join my team on Nandi:\n\nwss://relay.acme.test/relay#join=acme%202026'
          ),
      },
      configurable: true,
    })
    render(<ConnectionView />)
    await user.click(
      await screen.findByRole('button', { name: 'Paste invite' })
    )
    await waitFor(() =>
      expect(screen.getByLabelText('Relay URL')).toHaveValue(
        'wss://relay.acme.test/relay'
      )
    )
    expect(screen.getByLabelText('Join code')).toHaveValue('acme 2026')
  })

  const PUBLIC = 'wss://relay.vaultdb.ai/relay'

  it('offers the public relay to people who skipped it', async () => {
    disconnected({
      getRelayStatus: ok(
        relayStatusFixture({
          connected: false,
          state: 'disconnected',
          relayUrl: null,
          relayUrlSource: null,
          publicRelayUrl: PUBLIC,
        })
      ),
      getPublicRelayOffer: ok({
        kind: 'ready',
        relay: PUBLIC,
        email: 'ada@example.com',
      }),
    })
    const user = userEvent.setup()
    render(<ConnectionView />)
    const card = await screen.findByTestId('public-relay-card')
    // Nothing asks the relay until the person opens it
    expect(commands.getPublicRelayOffer).not.toHaveBeenCalled()
    expect(card).toHaveTextContent('you share a group with')
    await user.click(
      screen.getByRole('button', { name: 'Join the public relay' })
    )
    expect(
      await screen.findByText(/You join as ada@example.com/)
    ).toBeInTheDocument()
  })

  it('shows the account, its Nandis, the limits and today on the public relay', async () => {
    const resetsAt = Date.UTC(2026, 10, 11)
    disconnected({
      getRelayStatus: ok(
        relayStatusFixture({
          relayUrl: PUBLIC,
          publicRelayUrl: PUBLIC,
          visibility: 'groups',
        })
      ),
      getPublicRelayAccount: ok({
        kind: 'account',
        relay: PUBLIC,
        thisInstance: 'ada-laptop-1a2b3c4d',
        account: {
          account: 'google:583231',
          provider: 'google',
          email: 'octocat@example.com',
          name: 'Octo Cat',
          createdAt: 1,
          instances: ['ada-laptop-1a2b3c4d', 'ada-desktop-9f8e7d6c'],
          limits: {
            groupsPerPerson: 10,
            membersPerGroup: 50,
            messagesPerDay: 500,
          },
          usage: { groups: 2, messagesToday: 37, resetsAt },
        },
      }),
    })
    const user = userEvent.setup()
    render(<ConnectionView />)
    const card = await screen.findByTestId('public-relay-account')
    expect(
      await screen.findByTestId('public-relay-joined-as')
    ).toHaveTextContent('Joined as octocat@example.com')
    expect(screen.getByTestId('public-relay-nandis')).toHaveTextContent(
      'ada-desktop-9f8e7d6c'
    )
    expect(screen.getByText('(this Nandi)')).toBeInTheDocument()
    const usage = screen.getByTestId('public-relay-usage')
    expect(usage).toHaveTextContent('2 of 10')
    expect(usage).toHaveTextContent('Up to 50')
    expect(usage).toHaveTextContent('37 of 500')
    expect(screen.getByTestId('public-relay-resets')).toHaveTextContent(
      "Today's messages reset"
    )
    expect(card).toHaveTextContent(
      'On a public relay you see only people you share a group with.'
    )
    expect(screen.queryByTestId('public-relay-card')).toBeNull()
    await user.click(screen.getByRole('button', { name: 'Open Groups' }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'network',
      tab: 'groups',
    })
  })

  it('names the account calmly while the public relay is away', async () => {
    disconnected({
      getRelayStatus: ok(
        relayStatusFixture({
          connected: false,
          state: 'reconnecting',
          relayUrl: PUBLIC,
          publicRelayUrl: PUBLIC,
        })
      ),
      getPublicRelayAccount: ok({
        kind: 'unavailable',
        relay: PUBLIC,
        account: 'google:1',
        email: 'grace@example.com',
        name: 'Grace',
        detail: "Couldn't reach it",
      }),
    })
    render(<ConnectionView />)
    expect(
      await screen.findByTestId('public-relay-joined-as')
    ).toHaveTextContent('Joined as grace@example.com')
    expect(screen.getByTestId('public-relay-account')).toHaveTextContent(
      "Your account's details show when the public relay answers."
    )
  })
})
