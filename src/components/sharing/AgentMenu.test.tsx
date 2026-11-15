import { beforeEach, describe, expect, it } from 'vitest'
import userEvent from '@testing-library/user-event'
import { fireEvent, render, screen, waitFor } from '@/test/test-utils'
import { AgentsListView } from '@/components/views/AgentsListView'
import { sharingOf } from '@/hooks/useAgentSharing'
import { agentCard, ok, sharedAgent } from '@/hooks/useSharedAgents.fixtures'
import { commands } from '@/lib/tauri-bindings'
import { useAgentStore } from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'
import { agentInfo } from '@/test/fixtures'

const delegation = {
  allowOutbound: false,
  acceptInbound: false,
  maxConcurrent: null,
  overrides: [],
}

/** A menu item by its i18n key or English label */
const item = (role: string, key: string, en: string) =>
  screen.findByRole(role as 'menuitem', {
    name: new RegExp(`(${key.replace('.', '\\.')}|${en})`),
  })

describe('sharingOf', () => {
  it('reads the relay from the "shared by me" entry', () => {
    const papers = sharedAgent(agentCard('me', 'papers'))
    expect(sharingOf([papers], 'papers')).toEqual({
      relay: true,
      shared: papers,
    })
  })

  it('is unshared when the agent is missing', () => {
    expect(sharingOf([], 'papers')).toEqual({
      relay: false,
      shared: undefined,
    })
    expect(sharingOf(undefined, 'papers').relay).toBe(false)
  })
})

describe('AgentMenu', () => {
  beforeEach(() => {
    useAgentStore.setState(useAgentStore.getInitialState())
    useNavigationStore.setState(useNavigationStore.getInitialState())
    Object.assign(commands, {
      listAgents: ok([agentInfo('papers', 'Papers')]),
      listAgentsSharedByMe: ok([]),
      getAgentDelegation: ok(delegation),
      setAgentDelegation: ok(null),
    })
  })

  const openByRightClick = async () => {
    // The menu lives on the Agents page's cards
    render(<AgentsListView />)
    const row = await screen.findByText('Papers')
    const event = fireEvent.contextMenu(row)
    // The row handled it: the browser menu (and its OS Share) never shows
    expect(event).toBe(false)
  }

  it('opens on right-click with relay sharing, no OS share', async () => {
    await openByRightClick()
    expect(
      await item('menuitemcheckbox', 'agentMenu.relay', 'relay')
    ).toBeInTheDocument()
  })

  it('turns relay delegation on, keeping other settings', async () => {
    const user = userEvent.setup()
    await openByRightClick()
    await user.click(await item('menuitemcheckbox', 'agentMenu.relay', 'relay'))
    expect(
      await screen.findByText('Let everyone on your relay use Papers?')
    ).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Share' }))
    await waitFor(() =>
      expect(commands.setAgentDelegation).toHaveBeenCalledWith('papers', {
        ...delegation,
        acceptInbound: true,
      })
    )
  })

  it("can't share a marketplace agent", async () => {
    Object.assign(commands, {
      listAgents: ok([
        agentInfo('papers', 'Papers', {
          origin: { kind: 'publisher', publisher: 'someone' },
        }),
      ]),
    })
    await openByRightClick()
    expect(
      await item('menuitemcheckbox', 'agentMenu.relay', 'relay')
    ).toHaveAttribute('data-disabled')
  })
})
