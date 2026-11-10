import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { openUrl } from '@tauri-apps/plugin-opener'
import { render, screen } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { ok } from '@/hooks/useSharedAgents.fixtures'
import { PublicRelayJoin } from './PublicRelayJoin'

const RELAY = 'wss://relay.vaultdb.ai/relay'

const started = {
  kind: 'started',
  signinId: 's-1',
  provider: 'github',
  userCode: 'WDJB-MJHT',
  verificationUri: 'https://github.com/login/device',
  verificationUriComplete: null,
  expiresIn: 900,
  interval: 1,
}

const ready = (providers = ['github', 'google', 'microsoft']) =>
  ok({ kind: 'ready', relay: RELAY, providers })

describe('PublicRelayJoin', () => {
  it('offers the providers the relay lists', async () => {
    Object.assign(commands, {
      getPublicRelayOffer: ready(['github', 'google']),
    })
    render(<PublicRelayJoin />)
    expect(
      await screen.findByRole('button', { name: 'Sign in with GitHub' })
    ).toBeInTheDocument()
    expect(
      screen.getByRole('button', { name: 'Sign in with Google' })
    ).toBeInTheDocument()
    expect(
      screen.queryByRole('button', { name: 'Sign in with Microsoft' })
    ).toBeNull()
    expect(screen.getByText(/500 messages a day/)).toBeInTheDocument()
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
        data: { kind: 'ready', relay: RELAY, providers: ['github'] },
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
      await screen.findByRole('button', { name: 'Sign in with GitHub' })
    ).toBeInTheDocument()
  })

  it('shows the code, opens the page, polls and ends signed in', async () => {
    const user = userEvent.setup()
    const onLinked = vi.fn()
    const poll = vi
      .fn()
      .mockResolvedValueOnce({
        status: 'ok',
        data: { kind: 'pending', interval: 1 },
      })
      .mockResolvedValueOnce({
        status: 'ok',
        data: {
          kind: 'linked',
          relay: RELAY,
          account: 'github:583231',
          provider: 'github',
          display: 'octocat',
        },
      })
    Object.assign(commands, {
      getPublicRelayOffer: ready(),
      startPublicRelaySignIn: ok(started),
      pollPublicRelaySignIn: poll,
    })
    render(<PublicRelayJoin onLinked={onLinked} />)
    await user.click(
      await screen.findByRole('button', { name: 'Sign in with GitHub' })
    )
    expect(
      await screen.findByTestId('public-relay-user-code')
    ).toHaveTextContent('WDJB-MJHT')
    expect(screen.getByTestId('public-relay-waiting')).toHaveTextContent(
      'Waiting for you to approve'
    )
    // Nothing opens until asked
    expect(openUrl).not.toHaveBeenCalledWith('https://github.com/login/device')
    await user.click(screen.getByRole('button', { name: 'Open sign-in page' }))
    expect(openUrl).toHaveBeenCalledWith('https://github.com/login/device')

    expect(
      await screen.findByTestId('public-relay-linked', {}, { timeout: 4000 })
    ).toHaveTextContent('Signed in as octocat with GitHub')
    expect(poll).toHaveBeenCalledTimes(2)
    expect(poll).toHaveBeenCalledWith('github', 's-1')
    expect(onLinked).toHaveBeenCalledWith(RELAY)
  })

  it('says why the relay refused, and offers the providers again', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getPublicRelayOffer: ready(['github']),
      startPublicRelaySignIn: ok({
        kind: 'refused',
        refusal: 'deviceLimit',
        message: 'An account links at most 10 Nandis on this relay',
      }),
    })
    render(<PublicRelayJoin />)
    await user.click(
      await screen.findByRole('button', { name: 'Sign in with GitHub' })
    )
    const failed = await screen.findByTestId('public-relay-failed')
    expect(failed).toHaveTextContent(
      'This account links as many Nandis as the public relay allows.'
    )
    expect(failed).toHaveTextContent('at most 10 Nandis')
    expect(
      screen.getByRole('button', { name: 'Sign in with GitHub' })
    ).toBeEnabled()
  })

  it('keeps waiting through a poll the relay missed', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      getPublicRelayOffer: ready(['github']),
      startPublicRelaySignIn: ok(started),
      pollPublicRelaySignIn: ok({ kind: 'unreachable', detail: 'reset' }),
    })
    render(<PublicRelayJoin />)
    await user.click(
      await screen.findByRole('button', { name: 'Sign in with GitHub' })
    )
    expect(
      await screen.findByText(
        "The relay didn't answer just now; still waiting…",
        {},
        { timeout: 3000 }
      )
    ).toBeInTheDocument()
    expect(screen.getByTestId('public-relay-user-code')).toBeInTheDocument()
  })
})
