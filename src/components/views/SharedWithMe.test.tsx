import { beforeEach, describe, expect, it } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { emitMockEvent } from '@/test/mock-events'
import { commands } from '@/lib/tauri-bindings'
import {
  agentCard,
  ok,
  providerOption,
  sharedAgent,
} from '@/hooks/useSharedAgents.fixtures'
import type { CapabilityProviders } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { groupByOwner } from '@/hooks/useSharedAgents'
import { SharedWithMeView } from './SharedWithMe'

const teamPapers = sharedAgent(
  agentCard('alice', 'papers', {
    name: 'Papers',
    capabilities: [{ name: 'find-papers' }, { name: 'summarize' }],
  }),
  { sources: ['team'], teamIds: ['acme'] }
)
const relayAtlas = sharedAgent(agentCard('bob', 'atlas', { name: 'Atlas' }), {
  sources: ['relay'],
})

/** `find-papers`: Atlas (auto, online) and Papers (registry only) */
const findPapers: CapabilityProviders = {
  capability: 'find-papers',
  providers: [
    providerOption(relayAtlas.card, {
      rank: 1,
      sources: ['relay'],
      teammate: true,
      stats: { calls: 4, succeeded: 3, failed: 1, avgDurationMs: 2500 },
      successRate: 0.75,
    }),
    providerOption(teamPapers.card, {
      online: false,
      sources: ['team'],
      teamIds: ['acme'],
      unavailable: 'offline',
    }),
  ],
  pinned: null,
  autoChoice: 'bob/atlas',
}
const summarize: CapabilityProviders = {
  capability: 'summarize',
  providers: [providerOption(teamPapers.card, { rank: 1, sources: ['team'] })],
  pinned: null,
  autoChoice: 'alice/papers',
}

describe('SharedWithMeView', () => {
  beforeEach(() => {
    Object.assign(commands, {
      listAgentsSharedWithMe: ok([teamPapers, relayAtlas]),
      listAgentsSharedByMe: ok([]),
      delegateTask: ok('task-1'),
      listCapabilityProviders: ok([findPapers, summarize]),
      setProviderPin: ok(null),
    })
  })

  it('shows an agent a team shares, with owner, capabilities, and team', async () => {
    render(<SharedWithMeView />)
    const card = within(await screen.findByTestId('shared-agent-alice/papers'))
    expect(card.getByText('Papers')).toBeInTheDocument()
    // The owner's name, with the handle in the tooltip
    expect(card.getByTitle('@alice-owner')).toHaveTextContent('alice')
    expect(card.getByText('find-papers')).toBeInTheDocument()
    expect(card.getByText('summarize')).toBeInTheDocument()
    expect(card.getByText('acme')).toBeInTheDocument()
    // No backend installs team agents yet
    expect(
      card.getByRole('button', { name: /shared\.install|install locally/i })
    ).toBeDisabled()
  })

  it('points to the network connection when not connected and nothing is shared', async () => {
    Object.assign(commands, {
      listAgentsSharedWithMe: ok([]),
      getRelayStatus: ok({ connected: false }),
    })
    const user = userEvent.setup()
    render(<SharedWithMeView />)
    expect(
      await screen.findByText('No one is sharing with you yet')
    ).toBeInTheDocument()
    // No Teams page to send anyone to
    expect(screen.queryByRole('button', { name: /go to teams/i })).toBeNull()
    await user.click(screen.getByRole('button', { name: 'Go to Network' }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'network',
      tab: 'connection',
    })
  })

  it('points to people when connected and nothing is shared', async () => {
    Object.assign(commands, {
      listAgentsSharedWithMe: ok([]),
      getRelayStatus: ok({ connected: true }),
    })
    const user = userEvent.setup()
    render(<SharedWithMeView />)
    await user.click(await screen.findByRole('button', { name: 'Find people' }))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'network',
      tab: 'people',
    })
  })

  it('filters by where it comes from, and by the page search', async () => {
    const user = userEvent.setup()
    const { rerender } = render(<SharedWithMeView />)
    await screen.findByTestId('shared-agent-alice/papers')

    await user.selectOptions(screen.getByRole('combobox'), 'relay')
    expect(screen.queryByTestId('shared-agent-alice/papers')).toBeNull()
    expect(screen.getByTestId('shared-agent-bob/atlas')).toBeInTheDocument()

    await user.selectOptions(screen.getByRole('combobox'), 'all')
    rerender(<SharedWithMeView search="alice" />)
    expect(screen.getByTestId('shared-agent-alice/papers')).toBeInTheDocument()
    expect(screen.queryByTestId('shared-agent-bob/atlas')).toBeNull()
  })

  it('keeps only agents that can do the chosen thing', async () => {
    render(<SharedWithMeView canDo="summarize" />)
    expect(
      await screen.findByTestId('shared-agent-alice/papers')
    ).toBeInTheDocument()
    expect(screen.queryByTestId('shared-agent-bob/atlas')).toBeNull()
  })

  it('calls an agent directly with the chosen capability and input', async () => {
    const user = userEvent.setup()
    render(<SharedWithMeView />)
    const card = within(await screen.findByTestId('shared-agent-alice/papers'))
    await user.click(card.getByRole('button', { name: /shared\.call|^call$/i }))

    const dialog = within(await screen.findByRole('dialog'))
    await user.selectOptions(
      dialog.getByLabelText(/shared\.capability|capability/i),
      'summarize'
    )
    await user.type(dialog.getByRole('textbox'), 'llm agents')
    await user.click(dialog.getByRole('button', { name: /shared\.send|send/i }))

    await waitFor(() =>
      expect(commands.delegateTask).toHaveBeenCalledWith(
        'summarize',
        'llm agents',
        'alice/papers'
      )
    )
  })

  it('refetches when an agent comes online at the relay', async () => {
    render(<SharedWithMeView />)
    await screen.findByTestId('shared-agent-alice/papers')
    const late = sharedAgent(agentCard('dave', 'late', { name: 'Late' }), {
      sources: ['relay'],
    })
    Object.assign(commands, {
      listAgentsSharedWithMe: ok([teamPapers, relayAtlas, late]),
    })
    emitMockEvent('relay://agent_online', { card: late.card })
    expect(await screen.findByTestId('shared-agent-dave/late')).toBeVisible()
  })

  it('counts the agents serving a capability on its badge', async () => {
    render(<SharedWithMeView />)
    const card = within(await screen.findByTestId('shared-agent-bob/atlas'))
    // Atlas and Papers both serve find-papers
    expect(await card.findByText('2')).toBeInTheDocument()
  })

  it('shows who gets the work for the chosen capability, and pins one', async () => {
    const user = userEvent.setup()
    // The Library's Can do filter shows the Providers strip
    render(<SharedWithMeView canDo="find-papers" />)
    const capability = within(
      await screen.findByTestId('capability-find-papers')
    )
    const atlas = within(capability.getByTestId('provider-bob/atlas'))
    expect(atlas.getByText('#1')).toBeInTheDocument()
    expect(atlas.getByText('@bob-owner')).toBeInTheDocument()
    const papers = within(capability.getByTestId('provider-alice/papers'))
    expect(papers.getByText(/unavailable\.offline|Offline/)).toBeInTheDocument()

    await user.click(
      papers.getByRole('button', { name: /capabilities\.pin$|^pin$/i })
    )
    await waitFor(() =>
      expect(commands.setProviderPin).toHaveBeenCalledWith(
        'find-papers',
        'alice/papers'
      )
    )
  })

  it('sends a task to whoever Auto picks', async () => {
    const user = userEvent.setup()
    render(<SharedWithMeView canDo="find-papers" />)
    const capability = within(
      await screen.findByTestId('capability-find-papers')
    )
    await user.click(
      capability.getByRole('button', { name: /capabilities\.send|send task/i })
    )
    const dialog = within(await screen.findByRole('dialog'))
    const sendTo = dialog.getByLabelText(/shared\.sendTo|send to/i)
    await waitFor(() =>
      expect(sendTo).toHaveDisplayValue(/autoWillUse|will use Atlas/)
    )
    await user.click(
      dialog.getByRole('button', { name: /^(shared\.send|send)$/i })
    )
    await waitFor(() =>
      expect(commands.delegateTask).toHaveBeenCalledWith(
        'find-papers',
        null,
        null
      )
    )
  })
})

describe('SharedWithMeView grouping', () => {
  beforeEach(() => {
    Object.assign(commands, {
      listAgentsSharedWithMe: ok([
        teamPapers,
        relayAtlas,
        sharedAgent(agentCard('alice', 'notes', { name: 'Notes' }), {
          sources: ['relay'],
          lastCalledAt: Date.now() - 60_000,
        }),
      ]),
      listAgentsSharedByMe: ok([]),
      listCapabilityProviders: ok([]),
    })
  })

  it('groups agents by owner, with a link to the owner profile', async () => {
    const user = userEvent.setup()
    render(<SharedWithMeView />)
    await screen.findByTestId('shared-agent-alice/papers')
    const alice = screen.getByTestId('owner-group-alice-owner')
    expect(within(alice).getByText(/Agents: 2/)).toBeInTheDocument()
    expect(screen.getByTestId('owner-group-bob-owner')).toBeInTheDocument()
    await user.click(
      within(alice).getByRole('button', { name: /view profile/i })
    )
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'member-profile',
      instanceId: 'alice',
    })
  })

  it("hides and shows an owner's agents", async () => {
    const user = userEvent.setup()
    render(<SharedWithMeView />)
    await screen.findByTestId('shared-agent-alice/papers')
    const [toggle] = screen.getAllByRole('button', { name: /show or hide/i })
    if (!toggle) throw new Error('no group toggle')
    await user.click(toggle)
    expect(screen.queryByTestId('shared-agent-alice/papers')).toBeNull()
    expect(screen.getByTestId('owner-group-alice-owner')).toBeInTheDocument()
    await user.click(toggle)
    expect(screen.getByTestId('shared-agent-alice/papers')).toBeInTheDocument()
  })

  it('shows only recently used agents', async () => {
    const user = userEvent.setup()
    render(<SharedWithMeView />)
    await screen.findByTestId('shared-agent-alice/papers')
    await user.selectOptions(screen.getByRole('combobox'), 'recent')
    expect(screen.getByTestId('shared-agent-alice/notes')).toBeInTheDocument()
    expect(screen.queryByTestId('shared-agent-alice/papers')).toBeNull()
    expect(screen.queryByTestId('owner-group-bob-owner')).toBeNull()
  })
})

describe('groupByOwner', () => {
  it('groups case-insensitively by handle, else by instance, sorted by name', () => {
    const anon = sharedAgent(
      agentCard('zed', 'x', { owner: { githubHandle: '', displayName: '' } })
    )
    const upper = sharedAgent(
      agentCard('bob', 'y', {
        owner: { githubHandle: 'BOB-OWNER', displayName: 'bob' },
      })
    )
    const groups = groupByOwner([relayAtlas, teamPapers, upper, anon])
    expect(groups.map(g => [g.key, g.agents.length])).toEqual([
      ['alice-owner', 1],
      ['bob-owner', 2],
      ['zed', 1],
    ])
    expect(groups[0]?.instanceId).toBe('alice')
  })
})
