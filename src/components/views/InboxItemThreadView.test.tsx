import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import { client } from '@/lib/client'
import type { InboxThread, WorkItem } from '@/lib/bindings'
import { useAgentStore } from '@/store/agent-store'
import { InboxItemThreadView } from './InboxItemThreadView'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })
const mocks = () =>
  commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const NOW = Date.now()
const MIN = 60_000

const thread = (overrides: Partial<InboxThread>): InboxThread => ({
  conversationId: 'inbox:approval:ap1',
  kind: 'approval',
  title: 'shell',
  waiting: true,
  tools: ['inbox_approve', 'inbox_refuse'],
  ...overrides,
})

const approval = {
  id: 'ap1',
  agentId: 'writer',
  runId: 'r1',
  conversationId: 'agent:writer',
  tool: 'shell',
  args: { cmd: 'git push origin main' },
  createdAt: NOW - 2 * MIN,
  expiresAt: NOW + 12 * MIN,
}

const assignment = {
  taskId: 't9',
  callerInstance: 'i-dev',
  callerName: 'Dev Chechi',
  request: 'Send the SOC 1 evidence owners',
  status: 'open' as const,
  note: null,
  createdAt: NOW - 60 * MIN,
  dueAt: NOW - 120 * MIN,
  closedAt: null,
  links: [{ name: 'Evidence folder', url: 'https://drive.example.com/soc1' }],
}

const failed: WorkItem = {
  id: 'run-9',
  kind: 'request',
  title: 'Nightly digest',
  state: 'failed',
  owner: null,
  progress: { completedSteps: 0, totalSteps: 1, percent: 0 },
  nextAction: null,
  workflowId: null,
  rating: null,
  startedAt: NOW - 10 * MIN,
  updatedAt: NOW - MIN,
}

beforeEach(() => {
  useAgentStore.setState({ conversations: {} })
  Object.assign(commands, {
    listWork: ok([]),
    listPendingApprovals: ok([]),
    listPendingManualSteps: ok([]),
    getPendingClarifications: ok([]),
    listDismissedWork: ok([]),
    listGatewayHolds: ok([]),
    listGatewayAssignments: ok([]),
    listTrustRequests: ok([]),
    listRequestReturns: ok([]),
    getChiefOfStaffDigest: ok({ since: null, items: [], queue: [] }),
    approveAction: ok(null),
    dismissWork: ok(null),
    sendInboxMessage: ok({
      runId: 'run-1',
      messageId: 'm1',
      conversationTitle: null,
    }),
    getConversation: ok({
      conversationId: 'inbox:approval:ap1',
      messages: [],
      currentRun: null,
    }),
  })
})

describe('an Inbox item’s Ask chat', () => {
  it('opens the item’s thread, with its card on top as the shortcuts', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      openInboxThread: ok(thread({})),
      listPendingApprovals: ok([approval]),
    })
    render(<InboxItemThreadView conversationId="inbox:approval:ap1" />)
    expect(mocks().openInboxThread).toHaveBeenCalledWith('inbox:approval:ap1')
    const card = await screen.findByTestId('approval-card')
    expect(
      screen.getByRole('heading', {
        name: 'Run a command: git push origin main',
      })
    ).toBeInTheDocument()
    expect(card).toHaveTextContent(/Expires in 1[12] min/)

    // Reject asks first
    await user.click(within(card).getByRole('button', { name: 'Reject' }))
    expect(await screen.findByText('Reject this step?')).toBeInTheDocument()
    expect(mocks().approveAction).not.toHaveBeenCalled()
    await user.click(screen.getByRole('button', { name: 'Keep waiting' }))

    await user.click(within(card).getByRole('button', { name: 'Approve' }))
    await waitFor(() =>
      expect(mocks().approveAction).toHaveBeenCalledWith(
        'writer',
        'ap1',
        'approved'
      )
    )
  })

  it('sends the user’s words to the Chief of Staff in this thread', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      openInboxThread: ok(thread({})),
      listPendingApprovals: ok([approval]),
    })
    render(<InboxItemThreadView conversationId="inbox:approval:ap1" />)
    await screen.findByTestId('approval-card')
    await user.type(
      screen.getByRole('textbox'),
      'Why does it need to push?{Enter}'
    )
    await waitFor(() =>
      expect(mocks().sendInboxMessage).toHaveBeenCalledWith(
        'inbox:approval:ap1',
        'Why does it need to push?',
        expect.any(String)
      )
    )
    expect(mocks().approveAction).not.toHaveBeenCalled()
  })

  it('marks an assignment done with a note from its card', async () => {
    const user = userEvent.setup()
    const closeGatewayAssignment = ok({ ...assignment, status: 'done' })
    Object.assign(commands, {
      openInboxThread: ok(
        thread({
          conversationId: 'inbox:assignment:t9',
          kind: 'assignment',
          tools: ['inbox_done', 'inbox_refuse', 'inbox_reassign'],
        })
      ),
      listGatewayAssignments: ok([assignment]),
      closeGatewayAssignment,
    })
    render(<InboxItemThreadView conversationId="inbox:assignment:t9" />)
    const card = await screen.findByTestId('inbox-assignment')
    expect(card).toHaveTextContent('Dev Chechi assigned you this')
    expect(card).toHaveTextContent('Send the SOC 1 evidence owners')
    expect(card).toHaveTextContent(/Overdue by/)
    const openExternal = vi
      .spyOn(client, 'openExternal')
      .mockResolvedValue(undefined)
    await user.click(
      within(within(card).getByRole('list', { name: 'Attached' })).getByRole(
        'button',
        { name: 'Evidence folder' }
      )
    )
    expect(openExternal).toHaveBeenCalledWith('https://drive.example.com/soc1')
    await user.type(
      within(card).getByRole('textbox', { name: 'Note for them (optional)' }),
      'In the audit folder'
    )
    await user.click(within(card).getByRole('button', { name: 'Done' }))
    expect(closeGatewayAssignment).toHaveBeenCalledWith(
      't9',
      true,
      'In the audit folder'
    )
  })

  it('dismisses a failure from its card', async () => {
    const user = userEvent.setup()
    Object.assign(commands, {
      openInboxThread: ok(
        thread({
          conversationId: 'inbox:failed:run-9',
          kind: 'failed',
          title: 'Nightly digest',
          tools: ['inbox_approve', 'inbox_done'],
        })
      ),
      listWork: ok([failed]),
    })
    render(<InboxItemThreadView conversationId="inbox:failed:run-9" />)
    await screen.findByText('Nightly digest failed', { selector: 'button' })
    await user.click(screen.getByRole('button', { name: 'Dismiss' }))
    await waitFor(() =>
      expect(mocks().dismissWork).toHaveBeenCalledWith('run-9')
    )
  })

  it('says when nothing is left to decide, and keeps the chat', async () => {
    Object.assign(commands, {
      openInboxThread: ok(
        thread({ waiting: false, tools: [], title: 'shell' })
      ),
    })
    render(<InboxItemThreadView conversationId="inbox:approval:ap1" />)
    expect(await screen.findByTestId('inbox-thread-decided')).toHaveTextContent(
      'Nothing is left to decide here'
    )
    expect(screen.getByRole('heading', { name: 'shell' })).toBeInTheDocument()
    expect(screen.queryByTestId('approval-card')).toBeNull()
    expect(screen.getByRole('textbox')).toBeInTheDocument()
  })

  it('shows why the item couldn’t be opened', async () => {
    Object.assign(commands, {
      openInboxThread: vi.fn().mockResolvedValue({
        status: 'error',
        error: 'That approval wasn’t found',
      }),
    })
    render(<InboxItemThreadView conversationId="inbox:approval:gone" />)
    expect(
      await screen.findByText('Couldn’t open this item'.replace('’', "'"))
    ).toBeInTheDocument()
    expect(screen.getByText('That approval wasn’t found')).toBeInTheDocument()
  })
})
