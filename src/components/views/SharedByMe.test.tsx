import { beforeEach, describe, expect, it } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import {
  agentCard,
  notImplemented,
  ok,
  sharedAgent,
} from '@/hooks/useSharedAgents.fixtures'
import { useNavigationStore } from '@/store/navigation-store'
import { SharedByMeView } from './SharedByMe'

const papers = sharedAgent(agentCard('me', 'papers', { name: 'Papers' }), {
  callCount: 7,
  lastCalledAt: Date.now() - 60 * 60 * 1000,
})

const delegation = {
  allowOutbound: true,
  acceptInbound: true,
  maxConcurrent: 2,
  overrides: [],
}

describe('SharedByMeView', () => {
  beforeEach(() => {
    Object.assign(commands, {
      listAgentsSharedByMe: ok([papers]),
      listAgentsSharedWithMe: ok([]),
      getAgentDelegation: ok(delegation),
      setAgentDelegation: ok(null),
      listTrustPolicies: ok([]),
      listDelegationHistory: notImplemented(),
      getRelayStatus: ok({
        connected: false,
        instanceId: 'me',
        relayUrl: null,
        agentsOnline: 0,
      }),
      listOnlineAgents: ok([]),
    })
  })

  it('shows my shared agents with call counts', async () => {
    render(<SharedByMeView />)
    const card = within(await screen.findByTestId('my-shared-agent-papers'))
    expect(card.getByText('Papers')).toBeInTheDocument()
    expect(card.getByText(/7/)).toBeInTheDocument()
    expect(card.getAllByRole('switch')).toHaveLength(1)
  })

  const acceptsDelegation = () =>
    screen.getByRole('switch', {
      name: /shared\.byMe\.acceptsDelegation|accepts delegation/i,
    })

  it('keeps sharing when the question is cancelled', async () => {
    const user = userEvent.setup()
    render(<SharedByMeView />)
    await screen.findByTestId('my-shared-agent-papers')
    await user.click(acceptsDelegation())
    await user.click(
      within(await screen.findByRole('alertdialog')).getByRole('button', {
        name: 'Cancel',
      })
    )
    expect(commands.setAgentDelegation).not.toHaveBeenCalled()
  })

  it('stops accepting delegation, keeping the other settings', async () => {
    const user = userEvent.setup()
    render(<SharedByMeView />)
    await screen.findByTestId('my-shared-agent-papers')
    await user.click(acceptsDelegation())
    // Asks first, naming what happens
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog).toHaveTextContent('People who use it will lose access')
    await user.click(
      within(dialog).getByRole('button', { name: 'Stop sharing' })
    )
    await waitFor(() =>
      expect(commands.setAgentDelegation).toHaveBeenCalledWith('papers', {
        ...delegation,
        acceptInbound: false,
      })
    )
  })

  it('says "Accepts delegation" without relay wording when no relay is set', async () => {
    render(<SharedByMeView />)
    await screen.findByTestId('my-shared-agent-papers')
    expect(
      await screen.findByRole('switch', {
        name: /^(shared\.byMe\.acceptsDelegation|accepts delegation)$/i,
      })
    ).toBeInTheDocument()
    expect(screen.queryByText(/\(relay\)/i)).toBeNull()
  })

  it('names the relay once a relay URL is set', async () => {
    Object.assign(commands, {
      getRelayStatus: ok({
        connected: true,
        instanceId: 'me',
        relayUrl: 'wss://relay.test/relay',
        agentsOnline: 1,
      }),
    })
    render(<SharedByMeView />)
    expect(
      await screen.findByRole('switch', {
        name: /shared\.byMe\.acceptsDelegationRelay|accepts delegation \(relay\)/i,
      })
    ).toBeInTheDocument()
  })

  it('points to What you offer when nothing is shared', async () => {
    Object.assign(commands, { listAgentsSharedByMe: ok([]) })
    const user = userEvent.setup()
    render(<SharedByMeView />)
    expect(
      await screen.findByText(
        /shared\.byMe\.emptyTitle|You aren't sharing any agents/i
      )
    ).toBeInTheDocument()
    await user.click(
      screen.getByRole('button', {
        name: /network\.sharing\.offer|what you offer/i,
      })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'network',
      tab: 'sharing',
      sharing: 'offer',
    })
  })

  it('leaves the trust graph to Who may call you', async () => {
    render(<SharedByMeView />)
    await screen.findByTestId('my-shared-agent-papers')
    expect(screen.queryByRole('tab')).toBeNull()
  })
})
