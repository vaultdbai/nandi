import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  ChevronRight,
  CircleAlert,
  ClipboardList,
  ExternalLink,
  HeartHandshake,
  KeyRound,
  Lightbulb,
  MessageCircleQuestion,
  MessageSquareReply,
  ShieldAlert,
  Sparkles,
  UserRoundPlus,
  type LucideIcon,
} from 'lucide-react'
import { toast } from 'sonner'
import { toastRelayActionError } from '@/lib/toast-error'
import { ApprovalRequestCard } from '@/components/agent-console/ApprovalRequestCard'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import { KnowMeTodoCard } from '@/components/knowme/KnowMeTodoCard'
import { RetroCard } from './RetroCard'
import { ManualStepCard } from '@/components/work/ManualStepCard'
import { useNow } from '@/components/agents/task-utils'
import {
  useCloseGatewayAssignment,
  useDecideGatewayHold,
  useDismissWork,
  useInbox,
  useDismissRequestReturn,
  useTodos,
} from '@/hooks/useInbox'
import { useRetryRequest } from '@/hooks/useRequests'
import { useRetryRun } from '@/hooks/useWork'
import { requesterName, useRespondTrustRequest } from '@/hooks/useTrustRequests'
import type { TrustRequest } from '@/lib/bindings'
import { decideApproval } from '@/lib/agent-actions'
import { client } from '@/lib/client'
import { formatDuration } from '@/lib/format-time'
import { useViewOpener } from '@/hooks/useViewOpener'
import { workItemView } from '@/lib/work'
import { approveWorkflowStep, runWorkflow } from '@/lib/workflow-actions'
import { cn } from '@/lib/utils'
import {
  assignmentItem,
  inboxItemView,
  inboxRowLabel,
  type InboxItem,
} from './inbox-model'
import {
  doneSince,
  dueText,
  groupTodos,
  isTodo,
  sourceKey,
  todoState,
  todoTone,
} from './todo-model'
import {
  AddTodo,
  DoneRecently,
  HandToAgent,
  TodoMarker,
  TodoProgress,
} from './TodoParts'

/**
 * Everything waiting on the user, one calm line each; opening one opens its
 * Ask chat with Nandi, with the item's card (approve or
 * reject, answer, finish a manual step) on top as shortcuts. Recent
 * failures follow; they aren't counted in the badge.
 */
export function Inbox() {
  const { t } = useTranslation()
  const { items, count, isPending } = useInbox()
  const todos = useTodos().data ?? []
  const { open } = useViewOpener()
  // To-dos have their own grouped cards below
  const others = items.filter(
    item => !(item.kind === 'assignment' && isTodo(item.assignment))
  )
  const groups = groupTodos(todos)

  return (
    <section aria-label={t('inbox.title')} className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
          {count > 0 ? t('inbox.titleCount', { count }) : t('inbox.title')}
        </h2>
        <div className="flex items-center gap-3">
          <AddTodo />
          <Button
            variant="link"
            size="sm"
            className="h-auto p-0 text-xs"
            onClick={() => open({ kind: 'decisions' })}
          >
            {t('inbox.pastDecisions')}
          </Button>
        </div>
      </div>
      {groups.length > 0 && others.length > 0 && (
        // A long Inbox never hides the to-dos: one line to jump to them
        <nav
          aria-label={t('inbox.todo.jump')}
          className="flex flex-wrap items-center gap-2 text-xs"
          data-testid="todo-jump"
        >
          {groups.map(group => (
            <Button
              key={group.id}
              variant="outline"
              size="sm"
              className="h-6 px-2 text-xs"
              onClick={() =>
                document
                  .getElementById(`todo-group-${group.id}`)
                  ?.scrollIntoView?.({ behavior: 'smooth', block: 'start' })
              }
            >
              {t(`inbox.todo.group.${group.id}`)}
              <span className="text-muted-foreground tabular-nums">
                {group.todos.length}
              </span>
            </Button>
          ))}
        </nav>
      )}
      {isPending ? (
        <Skeleton className="h-20 w-full" />
      ) : others.length === 0 && groups.length === 0 ? (
        <p className="text-sm text-muted-foreground" data-testid="inbox-empty">
          {t('inbox.empty')}
        </p>
      ) : (
        others.length > 0 && (
          <ul className="flex flex-col gap-2">
            {others.map(item => (
              <li key={item.id}>
                <InboxRow item={item} />
              </li>
            ))}
          </ul>
        )
      )}
      {groups.map(group => (
        <section
          key={group.id}
          id={`todo-group-${group.id}`}
          aria-label={t(`inbox.todo.group.${group.id}`)}
          data-testid={`todo-group-${group.id}`}
          className="flex flex-col gap-2 pt-2"
        >
          <h3 className="text-xs font-medium text-muted-foreground">
            {t(`inbox.todo.group.${group.id}`)}
          </h3>
          <ul className="flex flex-col gap-2">
            {group.todos.map(todo => (
              <li key={todo.taskId}>
                <AssignmentRow item={assignmentItem(todo)} />
              </li>
            ))}
          </ul>
        </section>
      ))}
      <DoneRecently todos={doneSince(todos, 0)} />
    </section>
  )
}

const ROW_ICONS: Record<InboxItem['kind'], LucideIcon> = {
  noModel: KeyRound,
  approval: ShieldAlert,
  workflowApproval: ShieldAlert,
  gatewayHold: ShieldAlert,
  assignment: ClipboardList,
  clarification: MessageCircleQuestion,
  manualStep: ClipboardList,
  trustRequest: UserRoundPlus,
  failed: CircleAlert,
  requestReturn: MessageSquareReply,
  intake: Sparkles,
  knowMe: HeartHandshake,
  retro: Lightbulb,
}

/**
 * One Inbox item as one line: what it is and how long it has; opening it
 * opens its Ask chat (`inboxItemView`). Shared with the companion, whose
 * links go to the main window (`useViewOpener`).
 */
export function InboxRow({ item }: { item: InboxItem }) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const now = useNow(item.deadline !== null, 30_000)
  const label = inboxRowLabel(item)
  const Icon = ROW_ICONS[item.kind]
  const overdue = item.deadline !== null && item.deadline <= now
  const quiet =
    item.kind === 'requestReturn' ||
    item.kind === 'knowMe' ||
    (item.kind === 'retro' && item.retro.status !== 'pending')
  return (
    <button
      type="button"
      data-testid="inbox-row"
      onClick={() => open(inboxItemView(item))}
      className="flex w-full min-w-0 items-center gap-3 rounded-lg border bg-card px-4 py-2.5 text-start hover:bg-accent"
    >
      <Icon
        aria-hidden
        className={cn(
          'size-4 shrink-0',
          item.kind === 'failed' || overdue
            ? 'text-destructive'
            : quiet
              ? 'text-primary'
              : 'text-warning'
        )}
      />
      <span className="min-w-0 flex-1 truncate text-sm">
        {'text' in label ? label.text : t(label.key, label.values)}
      </span>
      {item.deadline !== null && (
        <span
          className={cn(
            'shrink-0 text-xs',
            overdue ? 'text-destructive' : 'text-muted-foreground'
          )}
        >
          {overdue
            ? t('inbox.assignment.overdue', {
                duration: formatDuration(now - item.deadline),
              })
            : t('inbox.expiresIn', {
                duration: formatDuration(item.deadline - now),
              })}
        </span>
      )}
      <ChevronRight
        aria-hidden
        className="size-4 shrink-0 text-muted-foreground rtl:rotate-180"
      />
    </button>
  )
}

/**
 * One Inbox item's card with its fix in place: the shortcuts on top of the
 * item's Ask chat (`InboxItemThreadView`)
 */
export function InboxItemCard({ item }: { item: InboxItem }) {
  const { t } = useTranslation()
  const opener = useViewOpener()
  switch (item.kind) {
    case 'noModel':
      return (
        <SimpleRow
          icon={<KeyRound className="text-warning" />}
          text={t('inbox.noModel')}
          action={t('inbox.connectModel')}
          onAction={() => opener.openSettings('models')}
        />
      )
    case 'approval':
      return (
        <div className="flex flex-col gap-1">
          <ApprovalRequestCard
            message={{
              id: item.approval.id,
              tool: item.approval.tool,
              args: item.approval.args,
              status: 'pending',
              createdAt: item.approval.createdAt,
              expiresAt: item.approval.expiresAt,
            }}
            agentName={item.agentName}
            onDecide={(approvalId, decision) =>
              decideApproval(item.approval.agentId, approvalId, decision)
            }
          />
          <Button
            size="sm"
            variant="link"
            className="h-auto self-start px-0"
            onClick={() =>
              opener.open({ kind: 'live-run', runId: item.approval.runId })
            }
          >
            {t('liveWork.openChat')}
          </Button>
        </div>
      )
    case 'workflowApproval':
      return <WorkflowApprovalRow item={item} />
    case 'gatewayHold':
      return <GatewayHoldRow item={item} />
    case 'assignment':
      return <AssignmentRow item={item} />
    case 'clarification':
      return (
        <SimpleRow
          icon={<MessageCircleQuestion className="text-warning" />}
          text={t('inbox.question', {
            question: item.clarification.question.question,
          })}
          action={t('inbox.answer')}
          onAction={() =>
            opener.open({
              kind: 'delegation-thread',
              taskId: item.clarification.taskId,
            })
          }
        />
      )
    case 'manualStep':
      return <ManualStepCard step={item.step} />
    case 'trustRequest':
      return <TrustRequestRow request={item.request} />
    case 'failed':
      return <FailedRow item={item} />
    case 'requestReturn':
      return <RequestReturnRow item={item} />
    case 'knowMe':
      return <KnowMeTodoCard recheck={item.recheck} />
    case 'retro':
      return <RetroCard item={item.retro} />
    case 'intake':
      return (
        <SimpleRow
          icon={<Sparkles className="text-warning" />}
          text={t('inbox.intake', { text: item.work.text })}
          action={t('inbox.review')}
          onAction={() =>
            opener.open({
              kind: 'intake-thread',
              conversationId: item.work.threadId,
            })
          }
        />
      )
  }
}

/** A request the user sent that came back: what they said, and Dismiss */
function RequestReturnRow({
  item,
}: {
  item: Extract<InboxItem, { kind: 'requestReturn' }>
}) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const dismiss = useDismissRequestReturn()
  const ret = item.ret
  const bad = ret.outcome === 'expired' || ret.outcome === 'failed'
  return (
    <Card className="flex-row items-start gap-3 px-4 py-3 shadow-none">
      <MessageSquareReply
        aria-hidden
        className={`mt-0.5 size-4 shrink-0 ${bad ? 'text-destructive' : 'text-primary'}`}
      />
      <button
        type="button"
        className="min-w-0 flex-1 text-start text-sm"
        onClick={() => open({ kind: 'delegation-thread', taskId: ret.taskId })}
      >
        <span className="font-medium hover:underline">
          {t(`inbox.return.${ret.outcome}`, { name: ret.fromName })}
        </span>
        {ret.summary && (
          <span className="mt-0.5 line-clamp-2 block text-muted-foreground">
            {ret.summary}
          </span>
        )}
        {ret.request && (
          <span className="mt-0.5 line-clamp-1 block text-xs text-muted-foreground">
            {t('inbox.return.asked', { request: ret.request })}
          </span>
        )}
      </button>
      <Button
        size="sm"
        variant="ghost"
        disabled={dismiss.isPending}
        onClick={() => dismiss.mutate(ret.taskId)}
      >
        {t('inbox.dismiss')}
      </Button>
    </Card>
  )
}

/**
 * Approve or reject (asks first) where it waits, a row with a second line
 * and a deadline; shared by held requests and workflow step approvals
 */
function DecisionRow({
  title,
  detail,
  deadline,
  onOpen,
  onDecide,
  confirmTitle,
  confirmDescription,
  rejectLabel,
}: {
  title: string
  detail: string | null
  deadline: number | null
  onOpen: () => void
  onDecide: (approve: boolean) => Promise<void>
  confirmTitle: string
  confirmDescription: string
  rejectLabel: string
}) {
  const { t } = useTranslation()
  const now = useNow(deadline !== null, 30_000)
  const [busy, setBusy] = useState(false)
  const [confirming, setConfirming] = useState(false)
  const decide = async (approve: boolean) => {
    setBusy(true)
    await onDecide(approve)
    setBusy(false)
  }

  return (
    <Card className="flex-row items-center gap-3 px-4 py-3 shadow-none">
      <ShieldAlert aria-hidden className="size-4 shrink-0 text-warning" />
      <button
        type="button"
        className="min-w-0 flex-1 text-start hover:underline"
        onClick={onOpen}
      >
        <span className="block text-sm break-words">{title}</span>
        {detail && (
          <span className="block truncate text-xs text-muted-foreground">
            {detail}
          </span>
        )}
        {deadline !== null && (
          <span className="block text-xs text-muted-foreground">
            {t('inbox.expiresIn', {
              duration: formatDuration(Math.max(0, deadline - now)),
            })}
          </span>
        )}
      </button>
      <Button
        size="sm"
        variant="outline"
        disabled={busy}
        onClick={() => setConfirming(true)}
      >
        {rejectLabel}
      </Button>
      <Button size="sm" disabled={busy} onClick={() => void decide(true)}>
        {t('inbox.approve')}
      </Button>
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{confirmTitle}</AlertDialogTitle>
            <AlertDialogDescription>
              {confirmDescription}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('inbox.retryConfirm.keep')}
            </AlertDialogCancel>
            <AlertDialogAction onClick={() => void decide(false)}>
              {rejectLabel}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  )
}

/** Another Nandi's request held for my approval (task 5) */
function GatewayHoldRow({
  item,
}: {
  item: Extract<InboxItem, { kind: 'gatewayHold' }>
}) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const decide = useDecideGatewayHold()
  const { hold } = item
  return (
    <DecisionRow
      title={
        hold.capability === 'assign-me'
          ? t('inbox.hold.titleAssign', { member: hold.callerName })
          : hold.capability === 'ask-me'
            ? t('inbox.hold.titleAsk', { member: hold.callerName })
            : hold.capability
              ? t('inbox.hold.titleCapability', {
                  member: hold.callerName,
                  capability: hold.capability,
                })
              : t('inbox.hold.title', { member: hold.callerName })
      }
      detail={hold.summary || null}
      deadline={hold.expiresAt}
      onOpen={() => open({ kind: 'request', requestId: hold.taskId })}
      onDecide={async approve => {
        await decide
          .mutateAsync(
            { taskId: hold.taskId, approve },
            {
              onSuccess: () =>
                toast.success(
                  t(approve ? 'inbox.hold.approved' : 'inbox.hold.declined', {
                    member: hold.callerName,
                  })
                ),
              onError: toastRelayActionError,
            }
          )
          .catch(() => undefined)
      }}
      confirmTitle={t('inbox.hold.confirm.title', { member: hold.callerName })}
      confirmDescription={t('inbox.hold.confirm.description', {
        member: hold.callerName,
      })}
      rejectLabel={t('inbox.hold.decline')}
    />
  )
}

/**
 * Work someone assigned to me: what they asked, the links they attached,
 * when it's due (or how overdue), and Done / Decline (asks first), each
 * with an optional note that goes back to them.
 *
 * One of my to-dos (`todo-model`) is the same card: its sentence, coloured
 * by kind and urgency (a reminder has its bell), where it came from, what
 * Nandi prepared for it, who is on it, and Done or Hand to an agent.
 */
function AssignmentRow({
  item,
}: {
  item: Extract<InboxItem, { kind: 'assignment' }>
}) {
  const { t, i18n } = useTranslation()
  const { open } = useViewOpener()
  const close = useCloseGatewayAssignment()
  const { assignment } = item
  const now = useNow(assignment.dueAt !== null, 30_000)
  const [note, setNote] = useState('')
  const [confirming, setConfirming] = useState(false)
  // A to-do's card stays light: the note only when asked for
  const [noting, setNoting] = useState(false)
  const member = assignment.callerName
  const overdue = assignment.dueAt !== null && assignment.dueAt <= now
  const todo = isTodo(assignment)
  const state = todoState(assignment)
  const finish = (done: boolean) =>
    close.mutate(
      { taskId: assignment.taskId, done, note: note.trim() || null },
      {
        onSuccess: () =>
          toast.success(
            todo
              ? t('inbox.todo.doneToast')
              : t(
                  done
                    ? 'inbox.assignment.doneToast'
                    : 'inbox.assignment.declined',
                  { member }
                )
          ),
        onError: toastRelayActionError,
      }
    )

  return (
    <Card
      className="flex-col gap-2 px-4 py-3 shadow-none"
      data-testid={todo ? 'inbox-todo' : 'inbox-assignment'}
      data-tone={todo ? todoTone(assignment, now) : undefined}
    >
      <div className="flex items-start gap-3">
        {todo ? (
          <TodoMarker todo={assignment} now={now} />
        ) : (
          <ClipboardList
            aria-hidden
            className={
              overdue
                ? 'mt-0.5 size-4 shrink-0 text-destructive'
                : 'mt-0.5 size-4 shrink-0 text-warning'
            }
          />
        )}
        <button
          type="button"
          className="min-w-0 flex-1 text-start hover:underline"
          onClick={() =>
            open(
              todo
                ? inboxItemView(item)
                : { kind: 'request', requestId: assignment.taskId }
            )
          }
        >
          {todo ? (
            <span className="block text-sm break-words whitespace-pre-wrap">
              {/* A reminder leads with its date */}
              {assignment.kind === 'reminder' && assignment.dueAt !== null
                ? `${new Date(assignment.dueAt).toLocaleDateString(
                    i18n.language,
                    { month: 'short', day: 'numeric' }
                  )} · ${assignment.request}`
                : assignment.request}
            </span>
          ) : (
            <>
              <span className="block text-sm break-words">
                {t('inbox.assignment.title', { member })}
              </span>
              <span className="line-clamp-4 block text-xs whitespace-pre-wrap text-muted-foreground">
                {assignment.request}
              </span>
            </>
          )}
          {assignment.dueAt !== null && (
            <DueLine due={assignment.dueAt} now={now} overdue={overdue} />
          )}
          {todo && assignment.source && (
            <span className="block text-xs text-muted-foreground">
              {t(sourceKey(assignment.source))}
            </span>
          )}
        </button>
      </div>
      {todo && <TodoProgress todo={assignment} state={state} />}
      {assignment.links.length > 0 && (
        <ul
          className="flex flex-wrap gap-1.5 ps-7"
          aria-label={t('inbox.assignment.links')}
        >
          {assignment.links.map(link => (
            <li key={link.url} className="min-w-0">
              <button
                type="button"
                title={link.url}
                className="inline-flex max-w-64 items-center gap-1 rounded-md border bg-muted/50 px-2 py-1 text-xs hover:bg-accent"
                onClick={() =>
                  client
                    .openExternal(link.url)
                    .catch((error: unknown) =>
                      toast.error(
                        error instanceof Error ? error.message : String(error)
                      )
                    )
                }
              >
                <span className="truncate">{link.name}</span>
                <ExternalLink
                  aria-hidden
                  className="size-3 shrink-0 text-muted-foreground"
                />
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex flex-wrap items-center gap-2 ps-7">
        {todo && !noting && (
          <Button
            variant="link"
            size="sm"
            className="h-auto p-0 text-xs text-muted-foreground"
            onClick={() => setNoting(true)}
          >
            {t('inbox.todo.withNote')}
          </Button>
        )}
        {(!todo || noting) && (
          <Input
            autoFocus={todo}
            value={note}
            onChange={event => setNote(event.target.value)}
            placeholder={t(
              todo
                ? 'inbox.todo.notePlaceholder'
                : 'inbox.assignment.notePlaceholder'
            )}
            aria-label={t(
              todo
                ? 'inbox.todo.notePlaceholder'
                : 'inbox.assignment.notePlaceholder'
            )}
            className="h-8 min-w-0 flex-1 text-sm"
          />
        )}
        {todo && !noting && <span className="flex-1" />}
        {todo ? (
          state !== 'working' && <HandToAgent todo={assignment} />
        ) : (
          <Button
            size="sm"
            variant="outline"
            disabled={close.isPending}
            onClick={() => setConfirming(true)}
          >
            {t('inbox.assignment.decline')}
          </Button>
        )}
        <Button
          size="sm"
          disabled={close.isPending}
          onClick={() => finish(true)}
        >
          {t('inbox.assignment.done')}
        </Button>
      </div>
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('inbox.assignment.confirm.title', { member })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('inbox.assignment.confirm.description', { member })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('inbox.retryConfirm.keep')}
            </AlertDialogCancel>
            <AlertDialogAction onClick={() => finish(false)}>
              {t('inbox.assignment.decline')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  )
}

/**
 * When an assignment or a to-do is due (`dueText`): by the day for a
 * date-only due, by the hour for one with a time
 */
function DueLine({
  due,
  now,
  overdue,
}: {
  due: number
  now: number
  overdue: boolean
}) {
  const { t, i18n } = useTranslation()
  const text = dueText(due, now)
  if (!text) return null
  const date = (at: number) =>
    new Date(at).toLocaleDateString(i18n.language, {
      weekday: 'short',
      month: 'short',
      day: 'numeric',
    })
  const words = (() => {
    switch (text.key) {
      case 'dueToday':
        return t('inbox.due.today')
      case 'dueTomorrow':
        return t('inbox.due.tomorrow')
      case 'dueOn':
        return t('inbox.assignment.dueOn', { date: date(text.on) })
      case 'overdueYesterday':
        return t('inbox.due.yesterday')
      case 'overdueDays':
        return t('inbox.due.overdueDays', { days: text.days })
      case 'dueIn':
        return t('inbox.assignment.dueIn', {
          duration: formatDuration(text.duration),
        })
      case 'overdueBy':
        return t('inbox.assignment.overdue', {
          duration: formatDuration(text.duration),
        })
    }
  })()
  return (
    <span
      className={
        overdue
          ? 'block text-xs text-destructive'
          : 'block text-xs text-muted-foreground'
      }
    >
      {words}
    </span>
  )
}

/** A workflow step waiting for approval, decided right here when the work
 * item names the step (task 5); otherwise opened for review */
function WorkflowApprovalRow({
  item,
}: {
  item: Extract<InboxItem, { kind: 'workflowApproval' }>
}) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const work = item.item
  const target = work.nextAction?.target
  const runId = target?.runId ?? null
  const stepId = target?.stepId ?? null
  if (!runId || !stepId || !work.workflowId) {
    return (
      <SimpleRow
        icon={<ShieldAlert className="text-warning" />}
        text={t('inbox.workflowApproval', { workflow: work.title })}
        action={t('inbox.review')}
        onAction={() => open(workItemView(work))}
      />
    )
  }
  const workflowId = work.workflowId
  return (
    <DecisionRow
      title={t('inbox.workflowApproval', { workflow: work.title })}
      detail={t('inbox.workflowStep', { step: stepId })}
      deadline={item.deadline}
      onOpen={() => open(workItemView(work))}
      onDecide={approve =>
        approveWorkflowStep(
          workflowId,
          runId,
          stepId,
          approve ? 'approved' : 'rejected'
        )
      }
      confirmTitle={t('inbox.workflowConfirm.title', { step: stepId })}
      confirmDescription={t('inbox.workflowConfirm.description')}
      rejectLabel={t('inbox.reject')}
    />
  )
}

/**
 * Someone this Nandi refused for trust: Trust them (no question asked, it's
 * what was requested) or Not now (hides it until they ask again)
 */
export function TrustRequestRow({ request }: { request: TrustRequest }) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const respond = useRespondTrustRequest()
  const name = requesterName(request, t('network.unnamed'))
  const answer = (answer: 'trust' | 'deny') =>
    respond.mutate(
      { instanceId: request.instanceId, answer },
      {
        onSuccess: () =>
          toast.success(
            t(
              answer === 'trust'
                ? 'inbox.trust.trusted'
                : 'inbox.trust.dismissed',
              { name }
            )
          ),
        onError: toastRelayActionError,
      }
    )

  return (
    <Card className="flex-row items-center gap-3 px-4 py-3 shadow-none">
      <UserRoundPlus aria-hidden className="size-4 shrink-0 text-warning" />
      <button
        type="button"
        className="min-w-0 flex-1 text-start hover:underline"
        onClick={() =>
          open({ kind: 'network', tab: 'sharing', sharing: 'callers' })
        }
      >
        <span className="block text-sm break-words">
          {t('inbox.trust.title', { name })}
        </span>
        <span className="block truncate text-xs text-muted-foreground">
          {t('inbox.trust.asked', {
            capability: request.capability,
            agent: request.agentName,
            count: request.attempts,
          })}
        </span>
      </button>
      <Button
        size="sm"
        variant="ghost"
        disabled={respond.isPending}
        onClick={() => answer('deny')}
      >
        {t('inbox.trust.notNow')}
      </Button>
      <Button
        size="sm"
        disabled={respond.isPending}
        onClick={() => answer('trust')}
      >
        {t('inbox.trust.trust', { name: request.memberName ?? name })}
      </Button>
    </Card>
  )
}

function FailedRow({ item }: { item: Extract<InboxItem, { kind: 'failed' }> }) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const retry = useRetryRequest()
  const retryRun = useRetryRun()
  const dismiss = useDismissWork()
  const [confirming, setConfirming] = useState(false)
  const work = item.item
  const isWorkflow = work.kind === 'workflow' && work.workflowId !== null

  const runAgain = () => {
    if (isWorkflow && work.workflowId) {
      void runWorkflow(work.workflowId)
      return
    }
    // A run Nandi started (a trigger's, the mail check): the next attempt,
    // in the same conversation. Its tools still ask for approval.
    if (work.kind === 'trigger') {
      retryRun.mutate(work.id, {
        onSuccess: () =>
          toast.success(t('inbox.retrying', { title: work.title })),
        onError: error => toast.error(error.message),
      })
      return
    }
    // A new attempt of the delegation (the provider still gates it)
    retry.mutate(work.id, {
      onError: toastRelayActionError,
    })
  }
  // A workflow run again may write or send things again: ask first
  const onRetry = () => (isWorkflow ? setConfirming(true) : runAgain())

  return (
    <Card className="flex-row items-center gap-3 px-4 py-3 shadow-none">
      <CircleAlert aria-hidden className="size-4 shrink-0 text-destructive" />
      <button
        type="button"
        className="min-w-0 flex-1 truncate text-start text-sm hover:underline"
        onClick={() => open(workItemView(work))}
      >
        {t('inbox.failed', { title: work.title })}
      </button>
      <Button
        size="sm"
        variant="ghost"
        disabled={dismiss.isPending}
        onClick={() => dismiss.mutate(work.id)}
      >
        {t('inbox.dismiss')}
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={retry.isPending || retryRun.isPending}
        onClick={onRetry}
      >
        {t('inbox.retry')}
      </Button>
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('inbox.retryConfirm.title', { title: work.title })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('inbox.retryConfirm.description')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('inbox.retryConfirm.keep')}
            </AlertDialogCancel>
            <AlertDialogAction onClick={runAgain}>
              {t('inbox.retry')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  )
}

function SimpleRow({
  icon,
  text,
  action,
  onAction,
}: {
  icon: React.ReactNode
  text: string
  action: string
  onAction: () => void
}) {
  return (
    <Card className="flex-row items-center gap-3 px-4 py-3 shadow-none">
      <span aria-hidden className="shrink-0 [&_svg]:size-4">
        {icon}
      </span>
      <p className="min-w-0 flex-1 text-sm break-words">{text}</p>
      <Button size="sm" onClick={onAction}>
        {action}
      </Button>
    </Card>
  )
}
