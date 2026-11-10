import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { InboxThread, KnowMeStatus } from '@/lib/bindings'
import { InboxItemThreadView } from '@/components/views/InboxItemThreadView'
import { useAgentStore } from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'
import { TellNandiAboutYou } from './TellNandiAboutYou'

const ok = (data: unknown) => vi.fn().mockResolvedValue({ status: 'ok', data })
const mocks = () =>
  commands as unknown as Record<string, ReturnType<typeof vi.fn>>

const NOW = Date.now()

const status = (overrides: Partial<KnowMeStatus> = {}): KnowMeStatus => ({
  todoOpen: true,
  recheck: false,
  shownAt: NOW - 60_000,
  answeredAt: null,
  threadId: 'inbox:knowMe:you',
  groups: [],
  recent: [],
  ...overrides,
})

const THREAD: InboxThread = {
  conversationId: 'inbox:knowMe:you',
  kind: 'knowMe',
  title: 'Getting to know you',
  waiting: true,
  tools: ['knowme_propose'],
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
    openInboxThread: ok(THREAD),
    getConversation: ok({
      conversationId: 'inbox:knowMe:you',
      messages: [],
      currentRun: null,
    }),
    getKnowMe: ok(status()),
    knowMeTodo: ok(status({ todoOpen: false })),
  })
})

describe('the know-me to-do', () => {
  it('opens as the Chief of Staff chat, with Later and Skip on top', async () => {
    const user = userEvent.setup()
    render(<InboxItemThreadView conversationId="inbox:knowMe:you" />)
    const todo = await screen.findByTestId('know-me-todo')
    expect(todo).toHaveTextContent(
      "Tell me about your work, and anything at home you'd like help with"
    )
    expect(screen.getByRole('textbox')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Later' }))
    await waitFor(() =>
      expect(mocks().knowMeTodo).toHaveBeenCalledWith('later')
    )
  })

  it('stays a chat with nothing on top when the to-do is closed', async () => {
    Object.assign(commands, { getKnowMe: ok(status({ todoOpen: false })) })
    render(<InboxItemThreadView conversationId="inbox:knowMe:you" />)
    expect(await screen.findByRole('textbox')).toBeInTheDocument()
    expect(screen.queryByTestId('know-me-todo')).toBeNull()
    expect(screen.queryByTestId('inbox-thread-decided')).toBeNull()
  })
})

describe('Tell Nandi about you (What you offer)', () => {
  it('opens the know-me chat any time and shows the latest changes', async () => {
    const user = userEvent.setup()
    const navigate = vi.spyOn(useNavigationStore.getState(), 'navigate')
    Object.assign(commands, {
      getKnowMe: ok(
        status({
          todoOpen: false,
          recent: [
            {
              at: NOW - 3_600_000,
              kind: 'knowMe',
              itemId: 'you',
              itemLabel: 'Getting to know you',
              decision: 'approved',
              decidedBy: 'Sam',
              reason: 'on the card: Sprint planning',
              chatId: 'inbox:knowMe:you',
              chatExists: true,
            },
          ],
        })
      ),
    })
    render(<TellNandiAboutYou />)
    expect(await screen.findByTestId('know-me-recent')).toHaveTextContent(
      'on the card: Sprint planning'
    )
    await user.click(screen.getByRole('button', { name: 'Open the chat' }))
    expect(navigate).toHaveBeenCalledWith({
      kind: 'inbox-thread',
      conversationId: 'inbox:knowMe:you',
    })
  })
})
