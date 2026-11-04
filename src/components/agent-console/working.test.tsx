import { describe, expect, it } from 'vitest'
import { render, screen } from '@/test/test-utils'
import type { AgentInfo, UiMessage } from '@/types/agent'
import { MessageList } from './MessageList'
import { isWorking, showsWorking, turnStartedAt } from './working'

const user = (over: Partial<UiMessage> = {}): UiMessage =>
  ({
    kind: 'user',
    id: 'u1',
    runId: null,
    createdAt: 0,
    content: 'What do I own?',
    attachments: [],
    ...over,
  }) as UiMessage

const agentText = (content: string): UiMessage => ({
  kind: 'agent',
  id: 'a1',
  runId: 'r1',
  createdAt: 1,
  content,
})

const approval = (status: 'pending' | 'approved'): UiMessage => ({
  kind: 'approval',
  id: 'ap1',
  runId: 'r1',
  createdAt: 2,
  tool: 'delegate_to_agent',
  args: {},
  status,
  expiresAt: null,
})

describe('isWorking', () => {
  it('is on while the run is queued, running or held for approval', () => {
    for (const status of ['queued', 'running', 'awaitingApproval'] as const) {
      expect(isWorking({ status }, [user()])).toBe(true)
    }
    expect(isWorking({ status: 'completed' }, [user()])).toBe(false)
    expect(isWorking(null, [user()])).toBe(false)
  })

  it('is on as soon as the message is sent, before the run exists', () => {
    expect(isWorking(null, [user({ pending: true })])).toBe(true)
  })
})

describe('showsWorking', () => {
  it('shows until the reply starts streaming', () => {
    expect(showsWorking([user()], true)).toBe(true)
    expect(showsWorking([user(), agentText('')], true)).toBe(true)
    expect(showsWorking([user(), agentText('You own')], true)).toBe(false)
  })

  it("hides while an approval waits for the user, and shows once it's decided", () => {
    expect(showsWorking([user(), approval('pending')], true)).toBe(false)
    expect(showsWorking([user(), approval('approved')], true)).toBe(true)
  })

  it('never shows when the assistant is idle', () => {
    expect(showsWorking([user()], false)).toBe(false)
  })
})

describe('turnStartedAt', () => {
  it("is the user's last message", () => {
    expect(
      turnStartedAt([
        user({ createdAt: 5 }),
        agentText('x'),
        user({ id: 'u2', createdAt: 9 }),
      ])
    ).toBe(9)
    expect(turnStartedAt([agentText('x')])).toBeUndefined()
  })
})

describe('MessageList working dots', () => {
  const agent = { id: 'chief-of-staff', name: 'Chief of Staff' } as AgentInfo

  it('shows the agent working after the user sends', () => {
    render(<MessageList messages={[user()]} assistant={agent} working />)
    expect(
      screen.getByRole('status', {
        name: 'Chief of Staff is working on your request',
      })
    ).toHaveTextContent('Working on it...')
  })

  it('says how long the turn has taken after a few seconds', () => {
    render(
      <MessageList
        messages={[user({ createdAt: Date.now() - 12_000 })]}
        assistant={agent}
        working
      />
    )
    expect(screen.getByRole('status')).toHaveTextContent('12s')
  })

  it('shows nothing extra when idle', () => {
    render(<MessageList messages={[user()]} assistant={agent} />)
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })
})
