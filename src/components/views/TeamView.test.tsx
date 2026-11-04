import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { AgentCard, AgentInfo, Team, TeamDetail } from '@/lib/bindings'
import { TeamView } from './TeamView'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })

const team: Team = {
  id: 'acme',
  name: 'acme',
  githubOrg: 'acme',
  registryRepo: 'github.com/acme/nandi-registry',
  createdAt: 0,
  source: { kind: 'git', identifier: 'github.com/acme/nandi-registry' },
  members: [
    { githubHandle: 'hubot', displayName: 'hubot', role: 'admin', joinedAt: 0 },
    {
      githubHandle: 'octocat',
      displayName: 'octocat',
      role: 'owner',
      joinedAt: 0,
    },
  ],
}

function card(id: string, owner: string): AgentCard {
  return {
    id: `inst-${owner}/${id}`,
    name: `Agent ${id}`,
    description: `Does ${id}`,
    instanceId: `inst-${owner}`,
    endpoint: '',
    capabilities: [{ name: 'draft' }],
    acceptsDelegation: true,
    maxConcurrent: 1,
    currentLoad: 0,
    owner: { githubHandle: owner, displayName: owner },
    signature: null,
    issuedAt: 0,
  } as AgentCard
}

const detail: TeamDetail = {
  team,
  agents: [card('writer', 'hubot'), card('papers', 'octocat')],
  packs: ['research'],
  lastSyncedAt: 1_700_000_000_000,
}

/** Button whose label is the i18n key or its English text */
const button = (
  key: string,
  en: RegExp,
  scope: Pick<typeof screen, 'getByRole'> = screen
) =>
  scope.getByRole('button', {
    name: new RegExp(`${key.replaceAll('.', '\\.')}|${en.source}`, 'i'),
  })

/** The header's "New team" (the empty state has one too) */
const firstCreateButton = () => {
  const [first] = screen.getAllByRole('button', {
    name: /teams\.create|new team/i,
  })
  if (!first) throw new Error('no create button')
  return first
}

describe('TeamView', () => {
  beforeEach(() => {
    Object.assign(commands, {
      listTeams: ok([team]),
      getTeam: ok(detail),
      createTeam: ok(team),
      joinTeam: ok(null),
      leaveTeam: ok(null),
      syncTeamRegistry: ok(null),
      shareAgent: ok(null),
      unshareAgent: ok(null),
      getTeamRoster: ok({ teamId: 'acme', members: [], fetchedAt: 0 }),
      listDelegationHistory: ok([]),
    })
    vi.mocked(commands.listAgents).mockResolvedValue({
      status: 'ok',
      data: [
        { id: 'papers', name: 'Papers', origin: { kind: 'local' } },
        {
          id: 'bought',
          name: 'Bought',
          origin: { kind: 'publisher', publisher: 'someone' },
        },
      ] as AgentInfo[],
    })
  })

  it('offers to create a team when there are none', async () => {
    Object.assign(commands, { listTeams: ok([]) })
    render(<TeamView />)
    expect(
      await screen.findByText(/teams\.empty\.title|No teams yet/i)
    ).toBeInTheDocument()
  })

  it('shows the team’s members, shared agents, and packs', async () => {
    render(<TeamView />)
    const writer = within(await screen.findByTestId('team-agent-writer'))
    expect(writer.getByText('Agent writer')).toBeInTheDocument()
    expect(writer.getByText('draft')).toBeInTheDocument()
    // Only my own shares can be withdrawn
    expect(writer.queryByRole('button')).toBeNull()
    const papers = within(screen.getByTestId('team-agent-papers'))
    await userEvent.click(button('teams.unshare', /stop sharing/, papers))
    await waitFor(() =>
      expect(commands.unshareAgent).toHaveBeenCalledWith('acme', 'papers')
    )

    expect(screen.getByText('research')).toBeInTheDocument()
    expect(screen.getByText('hubot')).toBeInTheDocument()
    expect(
      screen.getByText('github.com/acme/nandi-registry')
    ).toBeInTheDocument()
  })

  it('lists several teams and switches between them', async () => {
    const other: Team = {
      ...team,
      id: 'globex',
      name: 'globex',
      githubOrg: 'globex',
      members: team.members.slice(0, 1),
    }
    const third: Team = { ...team, id: 'initech', name: 'initech' }
    Object.assign(commands, { listTeams: ok([team, other, third]) })
    const user = userEvent.setup()
    render(<TeamView />)
    const nav = within(
      await screen.findByRole('navigation', {
        name: /teams\.list\.label|my teams/i,
      })
    )
    await nav.findByText('globex')
    expect(nav.getAllByRole('button')).toHaveLength(3)
    expect(nav.getByText('acme')).toBeInTheDocument()
    expect(nav.getByText('initech')).toBeInTheDocument()

    await user.click(nav.getByText('globex'))
    await waitFor(() => expect(commands.getTeam).toHaveBeenCalledWith('globex'))
    expect(nav.getByText('globex').closest('button')).toHaveAttribute(
      'aria-current',
      'page'
    )
  })

  it('shows each member’s GitHub handle next to the name', async () => {
    const named: Team = {
      ...team,
      members: [
        {
          githubHandle: 'octocat',
          displayName: 'Mona Lisa',
          role: 'owner',
          joinedAt: 0,
        },
      ],
    }
    Object.assign(commands, {
      listTeams: ok([named]),
      getTeam: ok({ ...detail, team: named }),
    })
    render(<TeamView />)
    const name = await screen.findByText('Mona Lisa')
    const row = name.closest('li')
    if (!row) throw new Error('no member row')
    expect(within(row).getByText('@octocat')).toBeInTheDocument()
  })

  it('syncs the registry', async () => {
    render(<TeamView />)
    await screen.findByTestId('team-agent-writer')
    await userEvent.click(button('teams.sync', /^sync$/))
    await waitFor(() =>
      expect(commands.syncTeamRegistry).toHaveBeenCalledWith('acme')
    )
  })

  it('creates a team from a registry URL', async () => {
    Object.assign(commands, { listTeams: ok([]) })
    render(<TeamView />)
    await screen.findByText(/teams\.empty\.title|No teams yet/i)
    await userEvent.click(firstCreateButton())
    const dialog = within(await screen.findByRole('dialog'))
    await userEvent.type(
      dialog.getByRole('textbox'),
      'https://git.example.com/acme/nandi-registry.git'
    )
    await userEvent.click(
      button('teams.createDialog.create', /create team/, dialog)
    )
    await waitFor(() =>
      expect(commands.createTeam).toHaveBeenCalledWith(
        'https://git.example.com/acme/nandi-registry.git'
      )
    )
  })

  it('shares one of my own agents', async () => {
    render(<TeamView />)
    await screen.findByTestId('team-agent-writer')
    await userEvent.click(button('teams.share', /share agent/))
    const dialog = within(await screen.findByRole('dialog'))
    // Marketplace installs aren't offered
    await dialog.findByRole('option', { name: 'Papers' })
    expect(dialog.queryByRole('option', { name: 'Bought' })).toBeNull()
    await userEvent.click(button('teams.shareDialog.submit', /^share$/, dialog))
    await waitFor(() =>
      expect(commands.shareAgent).toHaveBeenCalledWith('acme', 'papers')
    )
  })
})
