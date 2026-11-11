import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { ok } from '@/hooks/useSharedAgents.fixtures'
import { useNavigationStore } from '@/store/navigation-store'
import { PublicRelayJoin } from './PublicRelayJoin'

const RELAY = 'wss://relay.vaultdb.ai/relay'

const ready = () =>
  ok({ kind: 'ready', relay: RELAY, email: 'ada@example.com' })

describe('PublicRelayJoin', () => {
  beforeEach(() => {
    useNavigationStore.setState(useNavigationStore.getInitialState())
  })

  it('joins with the connected email in one click', async () => {
    const user = userEvent.setup()
    const onLinked = vi.fn()
    const join = vi.fn().mockResolvedValue({
      status: 'ok',
      data: {
        kind: 'joined',
        relay: RELAY,
        account: 'google:1099',
        email: 'ada@example.com',
        name: 'Ada',
      },
    })
    Object.assign(commands, {
      getPublicRelayOffer: ready(),
      joinPublicRelay: join,
    })
    render(<PublicRelayJoin onLinked={onLinked} />)
    expect(
      await screen.findByText(/You join as ada@example.com/)
    ).toBeInTheDocument()
    expect(screen.getByText(/500 messages a day/)).toBeInTheDocument()
    // No sign-in providers to pick
    expect(screen.queryByRole('button', { name: /Sign in with/ })).toBeNull()
    await user.click(
      screen.getByRole('button', { name: 'Join the public relay' })
    )
    expect(await screen.findByTestId('public-relay-linked')).toHaveTextContent(
      'Joined as ada@example.com'
    )
    expect(join).toHaveBeenCalledTimes(1)
    expect(onLinked).toHaveBeenCalledWith(RELAY)
  })

  it('without a connected email, points to Settings › Email', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getPublicRelayOffer: ok({ kind: 'needsEmail', relay: RELAY }),
    })
    render(<PublicRelayJoin />)
    expect(
      await screen.findByText(
        'Connect your Outlook or Gmail in Settings › Email to join groups on the public relay.'
      )
    ).toBeInTheDocument()
    expect(
      screen.queryByRole('button', { name: 'Join the public relay' })
    ).toBeNull()
    await user.click(
      screen.getByRole('button', { name: 'Open Settings › Email' })
    )
    expect(useNavigationStore.getState().current).toMatchObject({
      kind: 'settings',
      section: 'email',
    })
  })

  it('a mailbox that needs reconnecting says so', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getPublicRelayOffer: ready(),
      joinPublicRelay: ok({
        kind: 'needsEmail',
        message: 'Connect ada@example.com again in Settings › Email.',
      }),
    })
    render(<PublicRelayJoin />)
    await user.click(
      await screen.findByRole('button', { name: 'Join the public relay' })
    )
    expect(
      await screen.findByTestId('public-relay-needs-email')
    ).toHaveTextContent('Connect ada@example.com again')
  })

  it('shows a refusal in one calm line with the relay words', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getPublicRelayOffer: ready(),
      joinPublicRelay: ok({
        kind: 'refused',
        refusal: 'machineLimit',
        message: 'One computer joins at most 5 Nandis to this relay',
      }),
    })
    render(<PublicRelayJoin />)
    await user.click(
      await screen.findByRole('button', { name: 'Join the public relay' })
    )
    const alert = await screen.findByRole('alert')
    expect(alert).toHaveTextContent(
      'This computer has joined as many Nandis or accounts as the public relay allows.'
    )
    expect(alert).toHaveTextContent('at most 5 Nandis')
  })

  it('is calm when the public relay is unreachable, and can try again', async () => {
    const user = userEvent.setup()
    const offer = vi
      .fn()
      .mockResolvedValueOnce({
        status: 'ok',
        data: { kind: 'unreachable', relay: RELAY, detail: 'refused' },
      })
      .mockResolvedValueOnce({
        status: 'ok',
        data: { kind: 'ready', relay: RELAY, email: 'ada@example.com' },
      })
    Object.assign(commands, { getPublicRelayOffer: offer })
    render(<PublicRelayJoin />)
    const calm = await screen.findByTestId('public-relay-unreachable')
    expect(calm).toHaveTextContent(
      "The public relay isn't reachable right now — you can join later from Network › Connection."
    )
    expect(screen.queryByRole('alert')).toBeNull()
    await user.click(screen.getByRole('button', { name: 'Try again' }))
    expect(
      await screen.findByRole('button', { name: 'Join the public relay' })
    ).toBeInTheDocument()
  })
})
