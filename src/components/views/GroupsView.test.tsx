import { beforeEach, describe, expect, it } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { ok } from '@/hooks/useSharedAgents.fixtures'
import type {
  GroupSummary,
  GroupView,
  GroupsOverview,
  NetworkSnapshot,
} from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'
import { GroupsView } from './GroupsView'

const summary = (
  id: string,
  name: string,
  role: GroupSummary['role'] = null
): GroupSummary => ({
  id,
  name,
  description: `${name} work`,
  memberCount: 2,
  createdAt: 0,
  policyVersion: 1,
  role,
})

const overview = (partial: Partial<GroupsOverview> = {}): GroupsOverview => ({
  connected: true,
  visibility: 'company',
  groups: [],
  invites: [],
  policyTemplate: '---\nrequests:\n  approval: always\n---\n',
  ...partial,
})

const view = (partial: Partial<GroupView['policy']> = {}): GroupView => ({
  group: {
    summary: summary('g_1', 'Finance', 'admin'),
    members: [
      {
        instanceId: 'me',
        displayName: 'Me',
        role: 'admin',
        joinedAt: 0,
      },
      {
        instanceId: 'bob',
        displayName: 'Bob',
        role: 'member',
        joinedAt: 0,
      },
    ],
    policy: {
      version: 2,
      text: '---\nrequests:\n  approval: always\n---\nWhy.\n',
      author: 'me',
      signature: 'sig',
      updatedAt: 0,
    },
    invites: [],
  },
  policy: {
    verified: true,
    problem: null,
    version: 2,
    author: 'me',
    authorName: 'Me',
    approval: 'always',
    ...partial,
  },
  myRole: 'admin',
  myAccess: 'fully-automated',
  me: 'me',
})

const snapshot: NetworkSnapshot = {
  relayUrl: 'wss://relay.example',
  relayConnected: true,
  members: [],
  fetchedAt: 0,
}

describe('GroupsView', () => {
  beforeEach(() => {
    useNavigationStore.setState({
      current: { kind: 'network', tab: 'groups' },
      history: [],
    })
    Object.assign(commands, {
      listGroups: ok(overview()),
      getNetworkSnapshot: ok(snapshot),
    })
  })

  it('asks to connect when there is no relay, never an error', async () => {
    Object.assign(commands, {
      listGroups: ok(overview({ connected: false })),
    })
    render(<GroupsView />)
    expect(await screen.findByTestId('relay-connect-notice')).toBeTruthy()
  })

  it('lists my groups first, and invites with Accept', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      listGroups: ok(
        overview({
          groups: [summary('g_2', 'Alpha'), summary('g_1', 'Finance', 'admin')],
          invites: [
            {
              code: 'abc',
              groupId: 'g_3',
              groupName: 'Ops',
              instanceId: 'me',
              invitedBy: 'bob',
              createdAt: 0,
              expiresAt: null,
              usesLeft: 1,
            },
          ],
        })
      ),
      joinGroup: ok(view()),
    })
    render(<GroupsView />)
    const cards = await screen.findAllByTestId('group-card')
    expect(cards[0]).toHaveTextContent('Finance')
    expect(cards[1]).toHaveTextContent('Alpha')
    await user.click(
      screen.getByRole('button', { name: /accept|groups\.accept/i })
    )
    await waitFor(() => expect(commands.joinGroup).toHaveBeenCalledWith('abc'))
  })

  it('shows a verified policy, my level and the stricter-policy note', async () => {
    Object.assign(commands, { getGroup: ok(view()) })
    render(<GroupsView groupId="g_1" />)
    expect(await screen.findByTestId('group-policy')).toHaveTextContent(
      /Verified|groups\.verified/
    )
    expect(screen.getByTestId('group-policy-stricter')).toBeTruthy()
    expect(screen.queryByTestId('group-policy-unverified')).toBeNull()
  })

  it('warns when the policy could not be verified', async () => {
    Object.assign(commands, {
      getGroup: ok(view({ verified: false, problem: 'bad signature' })),
    })
    render(<GroupsView groupId="g_1" />)
    expect(await screen.findByTestId('group-policy-unverified')).toBeTruthy()
  })

  it('saves my access level', async () => {
    const user = userEvent.setup()
    const setGroupAccess = ok('none')
    Object.assign(commands, { getGroup: ok(view()), setGroupAccess })
    render(<GroupsView groupId="g_1" />)
    await screen.findByTestId('group-access')
    await user.click(
      screen.getByRole('radio', { name: /^none|groups\.access\.none$/i })
    )
    await waitFor(() =>
      expect(setGroupAccess).toHaveBeenCalledWith('g_1', 'none')
    )
  })
})
