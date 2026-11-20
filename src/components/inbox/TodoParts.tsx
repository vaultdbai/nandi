import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  Bell,
  BellRing,
  Bot,
  ClipboardList,
  ExternalLink,
  ListChecks,
  Plus,
  Undo2,
} from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { useAgents } from '@/hooks/useAgents'
import { useAgentNames } from '@/hooks/useHome'
import { useAddTodo, useHandTodo, useReopenTodo } from '@/hooks/useInbox'
import { useViewOpener } from '@/hooks/useViewOpener'
import { DEFAULT_AGENT } from '@/lib/bindings'
import type { GatewayAssignment, TodoKind } from '@/lib/bindings'
import { isDefaultAgent, ownAgents } from '@/lib/agent-identity'
import { rawError } from '@/lib/friendly-error'
import { cn } from '@/lib/utils'
import {
  reminderRings,
  relatedLink,
  TODO_TONE_TEXT,
  todoTone,
  type TodoState,
} from './todo-model'

/**
 * A to-do's icon: its kind's, a bell for a reminder (ringing near its
 * date), in its tone's colour
 */
export function TodoIcon({
  todo,
  now,
  className,
}: {
  todo: GatewayAssignment
  now: number
  className?: string
}) {
  const classes = cn('shrink-0', TODO_TONE_TEXT[todoTone(todo, now)], className)
  switch (todo.kind) {
    case 'reminder':
      return reminderRings(todo, now) ? (
        <BellRing aria-hidden className={classes} />
      ) : (
        <Bell aria-hidden className={classes} />
      )
    case 'check':
      return <ListChecks aria-hidden className={classes} />
    case 'do':
      return <ClipboardList aria-hidden className={classes} />
  }
}

/** The card's marker: the kind's icon in its urgency colour */
export function TodoMarker({
  todo,
  now,
}: {
  todo: GatewayAssignment
  now: number
}) {
  const { t } = useTranslation()
  return (
    <>
      <TodoIcon todo={todo} now={now} className="mt-0.5 size-4" />
      <span className="sr-only">
        {t(`inbox.todo.tone.${todoTone(todo, now)}`)}
      </span>
    </>
  )
}

/**
 * Who is on it (an agent's run going, or just ended: "mark done?"), and
 * what Nandi prepared for it, opening that item
 */
export function TodoProgress({
  todo,
  state,
}: {
  todo: GatewayAssignment
  state: TodoState
}) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const agentName = useAgentNames()
  const related = relatedLink(todo.relatedRef)
  const agent = todo.assignedAgent ? agentName(todo.assignedAgent) : null
  if (!related && !(agent && (state === 'working' || state === 'finished'))) {
    return null
  }
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 ps-7 text-xs">
      {agent && state === 'working' && (
        <span
          className="flex items-center gap-1 text-success"
          data-testid="todo-working"
        >
          <Bot aria-hidden className="size-3.5 animate-pulse" />
          {t('inbox.todo.working', { agent })}
        </span>
      )}
      {agent && state === 'finished' && (
        <span
          className="flex items-center gap-1 text-foreground"
          data-testid="todo-finished"
        >
          <Bot aria-hidden className="size-3.5 text-success" />
          {t('inbox.todo.finished', { agent })}
        </span>
      )}
      {related && (
        <button
          type="button"
          className="inline-flex items-center gap-1 text-primary hover:underline"
          onClick={() => open(related.view)}
        >
          {t(`inbox.todo.related.${related.label}`)}
          <ExternalLink aria-hidden className="size-3" />
        </button>
      )}
    </div>
  )
}

/**
 * Hand to an agent: Nandi first, then the person's agents that can run.
 * The agent gets "Please do this: …" in its chat, which opens.
 */
export function HandToAgent({ todo }: { todo: GatewayAssignment }) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const hand = useHandTodo()
  const { data: agents = [] } = useAgents()
  const nandi = agents.find(isDefaultAgent)
  // An agent that failed to load can't take work
  const others = ownAgents(agents).filter(agent => agent.status !== 'error')
  const give = (agentId: string) =>
    hand.mutate(
      { taskId: todo.taskId, agentId },
      {
        onSuccess: handed =>
          open(
            handed.agentId === DEFAULT_AGENT
              ? { kind: 'inbox-thread', conversationId: handed.conversationId }
              : { kind: 'agent', agentId: handed.agentId, tab: 'chat' }
          ),
        onError: error => toast.error(rawError(error)),
      }
    )
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" variant="outline" disabled={hand.isPending}>
          <Bot aria-hidden />
          {t('inbox.todo.hand')}
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onSelect={() => give(DEFAULT_AGENT)}>
          {nandi?.name ?? t('app.name')}
        </DropdownMenuItem>
        {others.length > 0 && <DropdownMenuSeparator />}
        {others.map(agent => (
          <DropdownMenuItem key={agent.id} onSelect={() => give(agent.id)}>
            {agent.name}
          </DropdownMenuItem>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

const KINDS: TodoKind[] = ['do', 'check', 'reminder']

/** "Add a to-do": the sentence, its kind and an optional date */
export function AddTodo() {
  const { t } = useTranslation()
  const add = useAddTodo()
  const [adding, setAdding] = useState(false)
  const [text, setText] = useState('')
  const [kind, setKind] = useState<TodoKind>('do')
  const [due, setDue] = useState('')
  if (!adding) {
    return (
      <Button
        variant="link"
        size="sm"
        className="h-auto p-0 text-xs"
        onClick={() => setAdding(true)}
      >
        <Plus aria-hidden className="size-3" />
        {t('inbox.todo.add')}
      </Button>
    )
  }
  const submit = () => {
    const sentence = text.trim()
    if (!sentence) return
    add.mutate(
      { text: sentence, kind, due: due || null },
      {
        onSuccess: () => {
          setText('')
          setDue('')
          setKind('do')
          setAdding(false)
        },
        onError: error => toast.error(rawError(error)),
      }
    )
  }
  return (
    <form
      className="flex w-full flex-wrap items-center gap-2"
      data-testid="add-todo"
      onSubmit={event => {
        event.preventDefault()
        submit()
      }}
    >
      <Input
        autoFocus
        value={text}
        onChange={event => setText(event.target.value)}
        placeholder={t('inbox.todo.addPlaceholder')}
        aria-label={t('inbox.todo.addPlaceholder')}
        className="h-8 min-w-48 flex-1 text-sm"
      />
      <NativeSelect
        value={kind}
        onChange={event => setKind(event.target.value as TodoKind)}
        aria-label={t('inbox.todo.kindLabel')}
        className="h-8 text-sm"
      >
        {KINDS.map(k => (
          <NativeSelectOption key={k} value={k}>
            {t(`inbox.todo.kind.${k}`)}
          </NativeSelectOption>
        ))}
      </NativeSelect>
      <Input
        type="date"
        value={due}
        onChange={event => setDue(event.target.value)}
        aria-label={t('inbox.todo.dueLabel')}
        className="h-8 w-auto text-sm"
      />
      <Button size="sm" type="submit" disabled={add.isPending || !text.trim()}>
        {t('inbox.todo.addButton')}
      </Button>
      <Button
        size="sm"
        variant="ghost"
        type="button"
        onClick={() => setAdding(false)}
      >
        {t('inbox.todo.cancel')}
      </Button>
    </form>
  )
}

/** To-dos done in the last week, collapsed, each with Undo */
export function DoneRecently({ todos }: { todos: GatewayAssignment[] }) {
  const { t } = useTranslation()
  const reopen = useReopenTodo()
  if (todos.length === 0) return null
  return (
    <Collapsible data-testid="todo-done-recently">
      <CollapsibleTrigger asChild>
        <Button
          variant="link"
          size="sm"
          className="h-auto p-0 text-xs text-muted-foreground"
        >
          {t('inbox.todo.doneRecently', { n: todos.length })}
        </Button>
      </CollapsibleTrigger>
      <CollapsibleContent>
        <ul className="mt-1 flex flex-col">
          {todos.map(todo => (
            <li
              key={todo.taskId}
              className="flex items-center gap-2 py-0.5 text-xs text-muted-foreground"
            >
              <span className="min-w-0 flex-1 truncate line-through">
                {todo.request}
              </span>
              <Button
                variant="ghost"
                size="sm"
                className="h-6 px-2 text-xs"
                disabled={reopen.isPending}
                onClick={() => reopen.mutate(todo.taskId)}
              >
                <Undo2 aria-hidden className="size-3" />
                {t('inbox.todo.undo')}
              </Button>
            </li>
          ))}
        </ul>
      </CollapsibleContent>
    </Collapsible>
  )
}
