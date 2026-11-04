import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { ok } from '@/hooks/useSharedAgents.fixtures'
import {
  gatewayStatusFixture,
  manifestFixture,
  relayStatusFixture,
} from '@/components/network/network-fixtures'
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
})
