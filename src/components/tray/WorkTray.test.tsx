import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, within } from '@/test/test-utils'
import type { HomeRow } from '@/components/home/home-model'
import type { InboxItem } from '@/components/inbox/inbox-model'
import { useNavigationStore } from '@/store/navigation-store'
import { useUIStore } from '@/store/ui-store'
import { WorkTray } from './WorkTray'

const NOW = 1_000_000_000

const mocks = vi.hoisted(() => ({
  feed: {
    inbox: { items: [] as InboxItem[] },
    running: [] as HomeRow[],
    recent: [] as HomeRow[],
  },
  decideApproval: vi.fn(),
}))

vi.mock('@/hooks/useHome', () => ({ useHomeFeed: () => mocks.feed }))
vi.mock('@/lib/agent-actions', () => ({
  decideApproval: mocks.decideApproval,
}))

const approval: InboxItem = {
  id: 'approval:a1',
  createdAt: NOW,
  deadline: null,
  kind: 'approval',
  agentName: 'Writer',
  approval: {
    id: 'a1',
    agentId: 'writer',
    runId: 'run-a1',
    conversationId: 'agent:writer',
    tool: 'shell',
    args: {},
    createdAt: NOW,
    expiresAt: null,
  },
}

const running: HomeRow = {
  id: 'work:w1',
  title: 'Weekly digest',
  status: 'running',
  source: { kind: 'workflow' },
  agent: null,
  progress: null,
  startedAt: NOW - 1000,
  endedAt: null,
  updatedAt: NOW - 1000,
  view: { kind: 'work-item', id: 'w1' },
  rating: null,
  retry: null,
}

describe('WorkTray', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    mocks.feed = { inbox: { items: [] }, running: [], recent: [] }
    useUIStore.setState({ rightSidebarVisible: true })
  })

  it('says each group is empty when nothing is going on', () => {
    render(<WorkTray />)
    expect(screen.getByTestId('tray-needsYou')).toHaveTextContent(
      'Nothing waits on you'
    )
    expect(screen.getByTestId('tray-working')).toHaveTextContent(
      'Nothing running'
    )
    expect(screen.getByTestId('tray-done')).toHaveTextContent(
      'Nothing finished today'
    )
  })

  it('decides a tool approval in place', async () => {
    const user = userEvent.setup()
    mocks.feed.inbox.items = [approval]
    render(<WorkTray />)
    const needsYou = screen.getByTestId('tray-needsYou')
    await user.click(within(needsYou).getByRole('button', { name: 'Approve' }))
    expect(mocks.decideApproval).toHaveBeenCalledWith(
      'writer',
      'a1',
      'approved'
    )
    await user.click(within(needsYou).getByRole('button', { name: 'Reject' }))
    expect(mocks.decideApproval).toHaveBeenLastCalledWith(
      'writer',
      'a1',
      'rejected'
    )
  })

  it('opens a row, a group and hides itself', async () => {
    const user = userEvent.setup()
    mocks.feed.running = [running]
    render(<WorkTray />)
    const working = screen.getByTestId('tray-working')
    await user.click(within(working).getByText('Weekly digest'))
    expect(useNavigationStore.getState().current).toEqual({
      kind: 'work-item',
      id: 'w1',
    })
    await user.click(
      within(screen.getByTestId('tray-needsYou')).getByRole('button', {
        name: /All/,
      })
    )
    expect(useNavigationStore.getState().current).toEqual({ kind: 'inbox' })
    await user.click(screen.getByRole('button', { name: 'Hide Work Tray' }))
    expect(useUIStore.getState().rightSidebarVisible).toBe(false)
  })
})
