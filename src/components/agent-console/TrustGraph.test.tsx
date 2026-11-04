import { beforeEach, describe, expect, it } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import {
  agentCard,
  notImplemented,
  ok,
  sharedAgent,
  task,
  team,
  trustRule,
} from '@/hooks/useSharedAgents.fixtures'
import { TrustGraph } from './TrustGraph'

const mine = sharedAgent(agentCard('me', 'papers', { name: 'Papers' }), {
  sources: ['relay', 'team'],
  teamIds: ['acme'],
})
const remote = sharedAgent(agentCard('bob', 'atlas', { name: 'Atlas' }), {
  sources: ['relay'],
})

describe('TrustGraph', () => {
  beforeEach(() => {
    Object.assign(commands, {
      listAgentsSharedByMe: ok([mine]),
      listAgentsSharedWithMe: ok([remote]),
      listTrustPolicies: ok([trustRule('carol/scribe', 'papers', 'allow')]),
      listDelegationHistory: ok([
        task('inbound', 'carol/scribe', 'papers', Date.now() - 1000),
        task('outbound', 'me/papers', 'bob/atlas', Date.now() - 1000),
      ]),
      listTeams: ok([team('acme')]),
      setTrustPolicy: ok(null),
    })
  })

  it('draws my agents, agents I call, callers, and teams', async () => {
    render(<TrustGraph />)
    expect(await screen.findByTestId('trust-node-me/papers')).toBeVisible()
    expect(await screen.findByTestId('trust-node-carol/scribe')).toBeVisible()
    expect(screen.getByTestId('trust-node-bob/atlas')).toBeVisible()
    expect(screen.getByTestId('trust-node-team:acme')).toBeVisible()
  })

  it('highlights a node’s connections and shows its details', async () => {
    const user = userEvent.setup()
    render(<TrustGraph />)
    await user.click(await screen.findByTestId('trust-node-carol/scribe'))
    const detail = within(screen.getByTestId('trust-node-detail'))
    expect(detail.getByText('Papers')).toBeInTheDocument()
    // Unconnected nodes fade
    expect(screen.getByTestId('trust-node-bob/atlas')).toHaveClass('opacity-30')
  })

  it('sets the policy for a caller from its edge', async () => {
    const user = userEvent.setup()
    render(<TrustGraph />)
    await user.click(
      await screen.findByTestId('trust-edge-carol/scribe->me/papers')
    )
    const detail = within(screen.getByTestId('trust-edge-detail'))
    expect(
      detail.getByRole('radio', {
        name: /shared\.graph\.policies\.allow|allow/i,
      })
    ).toHaveAttribute('aria-checked', 'true')
    await user.click(
      detail.getByRole('radio', { name: /shared\.graph\.policies\.deny|deny/i })
    )
    await waitFor(() =>
      expect(commands.setTrustPolicy).toHaveBeenCalledWith(
        'carol/scribe',
        'papers',
        'deny'
      )
    )
  })

  it('offers no policy on edges I call out on', async () => {
    const user = userEvent.setup()
    render(<TrustGraph />)
    await user.click(
      await screen.findByTestId('trust-edge-me/papers->bob/atlas')
    )
    expect(
      within(screen.getByTestId('trust-edge-detail')).queryByRole('radio')
    ).toBeNull()
  })

  it('still draws what it has when delegation and teams are unavailable', async () => {
    Object.assign(commands, {
      listDelegationHistory: notImplemented(),
      listTeams: notImplemented(),
      listTrustPolicies: ok([]),
    })
    render(<TrustGraph />)
    expect(await screen.findByTestId('trust-node-me/papers')).toBeVisible()
    expect(screen.queryByTestId('trust-node-carol/scribe')).toBeNull()
  })
})
