import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, within } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { GatewayAssignment } from '@/lib/bindings'
import { agentInfo } from '@/test/fixtures'
import { useNavigationStore } from '@/store/navigation-store'
import { Inbox } from './Inbox'

type Commands = Record<string, unknown>
const mocked = commands as unknown as Commands
const ok = <T,>(data: T) => vi.fn(async () => ({ status: 'ok' as const, data }))

const DAY = 24 * 60 * 60 * 1000

const todo = (
  overrides: Partial<GatewayAssignment> = {}
): GatewayAssignment => ({
  taskId: 'todo-1',
  callerInstance: '',
  callerName: 'Nandi',
  request: 'Reply to the landlord about the deposit. Deadline Friday.',
  status: 'open',
  note: null,
  createdAt: Date.now() - DAY,
  dueAt: Date.now() - DAY,
  closedAt: null,
  links: [],
  kind: 'do',
  source: 'email',
  sourceRef: 'msg-1',
  relatedRef: 'inbox:approval:a1',
  context: null,
  runId: null,
  assignedAgent: null,
  runStatus: null,
  ...overrides,
})

const KEYS = [
  'listTodos',
  'listGatewayAssignments',
  'listAgents',
  'closeGatewayAssignment',
  'reopenTodo',
  'handTodoToAgent',
  'addTodo',
] as const

describe('Inbox: the user’s to-dos', () => {
  const saved: Commands = {}
  beforeEach(() => {
    for (const key of KEYS) saved[key] = mocked[key]
  })
  afterEach(() => {
    for (const key of KEYS) mocked[key] = saved[key]
  })

  const serve = (todos: GatewayAssignment[]) => {
    mocked.listTodos = ok(todos)
    mocked.listGatewayAssignments = ok(todos.filter(t => t.status === 'open'))
  }

  it('groups them with their sentence, source, link and actions', async () => {
    serve([
      todo(),
      todo({
        taskId: 'todo-2',
        kind: 'check',
        request: 'Check whether the gas autopay is on',
        dueAt: null,
        relatedRef: null,
      }),
      todo({
        taskId: 'todo-3',
        kind: 'reminder',
        request: 'Policy renewal: review the offer',
        dueAt: Date.now() + 30 * DAY,
        relatedRef: null,
      }),
    ])
    render(<Inbox />)
    const doNow = await screen.findByTestId('todo-group-doNow')
    expect(doNow).toHaveTextContent('Do now')
    const card = within(doNow).getByTestId('inbox-todo')
    expect(card).toHaveAttribute('data-tone', 'overdue')
    expect(card).toHaveTextContent('Reply to the landlord about the deposit')
    expect(card).toHaveTextContent('From an email')
    expect(
      within(card).getByRole('button', {
        name: /Nandi has it queued for your approval/,
      })
    ).toBeInTheDocument()
    expect(
      within(card).getByRole('button', { name: /Hand to an agent/ })
    ).toBeInTheDocument()
    expect(
      within(card).getByRole('button', { name: 'Done' })
    ).toBeInTheDocument()
    // No Decline: nobody to answer
    expect(within(card).queryByRole('button', { name: /Decline/ })).toBeNull()

    expect(screen.getByTestId('todo-group-check')).toHaveTextContent(
      'Check and close'
    )
    const reminders = screen.getByTestId('todo-group-reminders')
    expect(within(reminders).getByTestId('inbox-todo')).toHaveAttribute(
      'data-tone',
      'reminder'
    )
  })

  it('shows what Nandi read it from, its details folded', async () => {
    const user = userEvent.setup()
    serve([
      todo({
        context: {
          from: 'Lena',
          title: 'Deposit',
          when: 'Mon 24 Nov',
          account: 'me@work.com',
          excerpt: 'Can you confirm by Friday?',
          why: 'The deposit comes back after the inspection',
        },
      }),
    ])
    render(<Inbox />)
    const card = await screen.findByTestId('inbox-todo')
    expect(card).toHaveTextContent('From an email · Lena · Deposit')
    expect(card).not.toHaveTextContent('Can you confirm by Friday?')
    await user.click(within(card).getByRole('button', { name: 'Details' }))
    const details = within(card).getByTestId('todo-context')
    expect(details).toHaveTextContent('Mon 24 Nov · To me@work.com')
    expect(details).toHaveTextContent('Can you confirm by Friday?')
    expect(details).toHaveTextContent(
      'Why it matters: The deposit comes back after the inspection'
    )
  })

  it('has no Details without a context', async () => {
    serve([todo()])
    render(<Inbox />)
    const card = await screen.findByTestId('inbox-todo')
    expect(within(card).queryByTestId('todo-context')).toBeNull()
  })

  it('marks one done, and Undo opens it again', async () => {
    const user = userEvent.setup()
    serve([
      todo(),
      todo({
        taskId: 'todo-9',
        status: 'done',
        closedAt: Date.now() - 60_000,
        request: 'Book the venue',
      }),
    ])
    const close = ok(todo({ status: 'done' }))
    const reopen = ok(todo())
    mocked.closeGatewayAssignment = close
    mocked.reopenTodo = reopen
    render(<Inbox />)
    const card = await screen.findByTestId('inbox-todo')
    await user.click(within(card).getByRole('button', { name: 'Done' }))
    expect(close).toHaveBeenCalledWith('todo-1', true, null)

    await user.click(
      screen.getByRole('button', { name: /Done recently \(1\)/ })
    )
    await user.click(screen.getByRole('button', { name: /Undo/ }))
    expect(reopen).toHaveBeenCalledWith('todo-9')
  })

  it('shows no note box until Done with a note', async () => {
    const user = userEvent.setup()
    serve([todo()])
    const close = ok(todo({ status: 'done' }))
    mocked.closeGatewayAssignment = close
    render(<Inbox />)
    const card = await screen.findByTestId('inbox-todo')
    expect(within(card).queryByRole('textbox')).toBeNull()
    await user.click(
      within(card).getByRole('button', { name: 'Done with a note' })
    )
    await user.type(within(card).getByRole('textbox'), 'Sent it')
    await user.click(within(card).getByRole('button', { name: 'Done' }))
    expect(close).toHaveBeenCalledWith('todo-1', true, 'Sent it')
  })

  it('says a date-only due by the day', async () => {
    const end = (days: number) => {
      const at = new Date()
      at.setDate(at.getDate() + days)
      at.setHours(23, 59, 59, 0)
      return at.getTime()
    }
    serve([
      todo({ taskId: 'a', dueAt: end(-2), relatedRef: null }),
      todo({ taskId: 'b', dueAt: end(0), relatedRef: null }),
      todo({ taskId: 'c', dueAt: end(1), relatedRef: null }),
    ])
    render(<Inbox />)
    const cards = await screen.findAllByTestId('inbox-todo')
    expect(cards[0]).toHaveTextContent('Overdue 2 days')
    expect(cards[1]).toHaveTextContent('Due today')
    expect(cards[2]).toHaveTextContent('Due tomorrow')
    expect(cards[0]).not.toHaveTextContent(/[0-9]+h/)
  })

  it('renders the Reminders group with bells and dates first, reachable from the top', async () => {
    const user = userEvent.setup()
    const day = (days: number) => {
      const at = new Date()
      at.setDate(at.getDate() + days)
      at.setHours(23, 59, 59, 0)
      return at.getTime()
    }
    serve([
      todo({
        taskId: 'r1',
        kind: 'reminder',
        request: 'Policy renewal',
        dueAt: day(1),
        relatedRef: null,
      }),
      todo({
        taskId: 'r2',
        kind: 'reminder',
        request: 'Dentist check-up',
        dueAt: day(40),
        relatedRef: null,
      }),
    ])
    // A long Inbox above them
    mocked.listGatewayAssignments = ok([
      todo({ taskId: 'r1', kind: 'reminder', dueAt: day(1) }),
      todo({ taskId: 'r2', kind: 'reminder', dueAt: day(40) }),
      ...Array.from({ length: 6 }, (_, i) =>
        todo({
          taskId: `p${i}`,
          source: null,
          callerInstance: 'desktop-sam',
          callerName: 'Sam Lee',
        })
      ),
    ])
    render(<Inbox />)
    const group = await screen.findByTestId('todo-group-reminders')
    expect(group).toHaveTextContent('Reminders')
    const cards = within(group).getAllByTestId('inbox-todo')
    expect(cards).toHaveLength(2)
    // Soonest first; the near one rings, the far one keeps the violet accent
    expect(cards[0]).toHaveAttribute('data-tone', 'soon')
    expect(cards[1]).toHaveAttribute('data-tone', 'reminder')
    expect(cards[1]).toHaveTextContent(/ · Dentist check-up/)
    expect(within(cards[1] as HTMLElement).getByText('Reminder')).toHaveClass(
      'sr-only'
    )
    const jump = screen.getByTestId('todo-jump')
    await user.click(within(jump).getByRole('button', { name: /Reminders/ }))
  })

  it('hands one to an agent and opens its chat', async () => {
    const user = userEvent.setup()
    serve([todo()])
    mocked.listAgents = ok([
      agentInfo('nandi', 'Nandi', { kind: 'default' }),
      agentInfo('writer', 'Writer', { kind: 'local' }),
      agentInfo('broken', 'Broken', { kind: 'local', status: 'error' }),
    ])
    const hand = ok({
      todo: todo({ runId: 'r1', assignedAgent: 'writer' }),
      agentId: 'writer',
      conversationId: 'agent:writer',
    })
    mocked.handTodoToAgent = hand
    render(<Inbox />)
    const card = await screen.findByTestId('inbox-todo')
    await user.click(
      within(card).getByRole('button', { name: /Hand to an agent/ })
    )
    expect(
      await screen.findByRole('menuitem', { name: 'Nandi' })
    ).toBeInTheDocument()
    expect(screen.queryByRole('menuitem', { name: 'Broken' })).toBeNull()
    await user.click(screen.getByRole('menuitem', { name: 'Writer' }))
    expect(hand).toHaveBeenCalledWith('todo-1', 'writer')
    await vi.waitFor(() =>
      expect(useNavigationStore.getState().current).toEqual({
        kind: 'agent',
        agentId: 'writer',
        tab: 'chat',
      })
    )
  })

  it('shows who is on one, and asks to mark it done once they finished', async () => {
    mocked.listAgents = ok([agentInfo('writer', 'Writer')])
    serve([
      todo({ runId: 'r1', assignedAgent: 'writer', runStatus: 'running' }),
      todo({
        taskId: 'todo-2',
        runId: 'r2',
        assignedAgent: 'writer',
        runStatus: 'completed',
      }),
    ])
    render(<Inbox />)
    expect(await screen.findByTestId('todo-working')).toHaveTextContent(
      'Writer is on it'
    )
    expect(screen.getByTestId('todo-finished')).toHaveTextContent(
      'Writer finished: mark done?'
    )
  })

  it('adds one by hand, with a kind and a date', async () => {
    const user = userEvent.setup()
    serve([])
    const add = ok(todo({ source: 'manual' }))
    mocked.addTodo = add
    render(<Inbox />)
    await user.click(await screen.findByRole('button', { name: /Add a to-do/ }))
    const form = screen.getByTestId('add-todo')
    await user.type(
      within(form).getByRole('textbox', { name: /What to do/ }),
      'Renew the passport'
    )
    await user.selectOptions(
      within(form).getByRole('combobox', { name: 'Kind' }),
      'reminder'
    )
    await user.click(within(form).getByRole('button', { name: 'Add' }))
    expect(add).toHaveBeenCalledWith('Renew the passport', 'reminder', null)
  })
})
