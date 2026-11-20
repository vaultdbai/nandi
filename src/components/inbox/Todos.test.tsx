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
