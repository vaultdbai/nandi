import { useTranslation } from 'react-i18next'
import type { TFunction } from 'i18next'
import {
  Activity,
  ArrowRightLeft,
  CircleHelp,
  CloudOff,
  Flag,
  MessageSquareReply,
  Send,
  ShieldAlert,
  StickyNote,
  UserRound,
  Waypoints,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { ClarificationCard } from '@/components/agent-console/ClarificationCard'
import { RouteReasons } from '@/components/network/RouteReasons'
import {
  DEFAULT_CLARIFICATION_POLICY,
  summarizePolicy,
} from '@/components/agents/clarification-policy'
import { EscalationApprovalCard } from '@/components/agent-console/EscalationApprovalCard'
import { formatJson } from '@/components/agent-console/format-json'
import { TaskStatePill } from '@/components/agents/task-display'
import {
  useAgentLabel,
  isUserRequester,
  useNow,
  useOfflineProvider,
} from '@/components/agents/task-utils'
import {
  isTerminalState,
  taskErrorMessage,
  useTask,
  useTaskEvents,
  useTaskMessages,
  useTaskPendingClarifications,
} from '@/hooks/useTasks'
import {
  USER_REQUESTER,
  useClarificationPolicy,
} from '@/hooks/useClarifications'
import type {
  JsonValue,
  PendingClarification,
  TaskMessage,
  TaskRecord,
} from '@/lib/bindings'
import { formatDuration } from '@/lib/format-time'
import { routeReasonsOf } from '@/lib/route-reasons'
import { formatCostUsd, formatTokenCount } from '@/lib/format-usage'
import { useNavigationStore } from '@/store/navigation-store'
import {
  asNumber,
  asRecord,
  buildThread,
  extractUsage,
  type ThreadItem,
} from '@/components/agents/delegation-thread-model'
import { ViewHeader } from './ViewHeader'

export interface DelegationThreadViewProps {
  taskId: string
}

/**
 * A remote requester is a card ID (`<instance>/<agent>`), and `_user` is
 * the user asking directly: neither is a local agent
 */
const isLocalAgent = (agentId: string) =>
  !agentId.includes('/') && !isUserRequester(agentId)

/** The requester as shown: "You" when the user asked directly */
function requesterName(
  task: TaskRecord,
  t: TFunction,
  agentLabel: (agent: string, instance: string) => string
): string {
  return isUserRequester(task.requesterAgent)
    ? t('tasks.you')
    : agentLabel(task.requesterAgent, task.requesterInstance)
}

/** One delegated task's thread: messages, events, clarifications */
export function DelegationThreadView({ taskId }: DelegationThreadViewProps) {
  const { t } = useTranslation()
  const task = useTask(taskId)
  const events = useTaskEvents(taskId)
  const messages = useTaskMessages(taskId)
  const pending = useTaskPendingClarifications(taskId)
  const offlineProvider = useOfflineProvider(task.data)

  if (task.isPending) {
    return (
      <div className="flex flex-col gap-3 p-4" aria-busy="true">
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-24 w-full" />
      </div>
    )
  }
  if (task.isError) {
    return (
      <div className="flex h-full flex-col">
        <ViewHeader title={t('delegationThread.title')} subtitle={taskId} />
        <p role="alert" className="p-4 text-sm text-destructive">
          {t('delegationThread.loadFailed', {
            error: taskErrorMessage(task.error),
          })}
        </p>
      </div>
    )
  }

  const record = task.data
  const thread = buildThread(record, events.data ?? [], messages.data ?? [])

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ThreadHeader task={record} />
      <div className="flex min-h-0 flex-1">
        <ScrollArea className="min-h-0 flex-1">
          <ol
            className="mx-auto flex max-w-3xl flex-col gap-3 p-4"
            aria-label={t('delegationThread.threadLabel')}
          >
            {thread.map((item, index) => (
              <li key={`${item.kind}-${index}`}>
                <ThreadEntry item={item} thread={thread} />
              </li>
            ))}
          </ol>
          {offlineProvider && (
            <p
              role="status"
              className="mx-auto flex max-w-3xl items-center gap-2 px-4 pb-4 text-sm text-muted-foreground"
              data-testid="thread-queued-offline"
            >
              <CloudOff className="size-4 shrink-0" />
              {t('tasks.queuedOffline', { name: offlineProvider })}
            </p>
          )}
          {(pending.data ?? []).length > 0 && (
            <div className="mx-auto flex max-w-3xl flex-col gap-3 px-4 pb-4">
              {(pending.data ?? []).map(clarification => (
                <PendingCard
                  key={`${clarification.taskId}-${clarification.question.id}`}
                  clarification={clarification}
                />
              ))}
            </div>
          )}
          {(events.isError || messages.isError) && (
            <p role="alert" className="px-4 pb-4 text-xs text-destructive">
              {t('delegationThread.partial', {
                error: taskErrorMessage(events.error ?? messages.error),
              })}
            </p>
          )}
        </ScrollArea>
        <ThreadSidePanel task={record} />
      </div>
    </div>
  )
}

function ThreadHeader({ task }: { task: TaskRecord }) {
  const { t, i18n } = useTranslation()
  const terminal = isTerminalState(task.state)
  const now = useNow(!terminal)
  const duration = (task.completedAt ?? now) - task.createdAt
  const usage = extractUsage(task.result)
  const agentLabel = useAgentLabel()

  return (
    <ViewHeader
      title={t('delegationThread.heading', { capability: task.capability })}
      titleAdornment={<TaskStatePill state={task.state} />}
      subtitle={
        <span className="flex flex-wrap items-center gap-x-3">
          <span dir="ltr" className="font-mono">
            {task.id}
          </span>
          <span>
            {t('delegationThread.parties', {
              requester: requesterName(task, t, agentLabel),
              provider: agentLabel(task.providerAgent, task.providerInstance),
            })}
          </span>
          <span className="tabular-nums">
            {t('delegationThread.duration', {
              duration: formatDuration(Math.max(0, duration)),
            })}
          </span>
          {usage.tokens !== null && (
            <span className="tabular-nums">
              {t('delegationThread.tokens', {
                tokens: formatTokenCount(usage.tokens, i18n.language),
              })}
            </span>
          )}
          {usage.costUsd !== null && (
            <span className="tabular-nums">
              {formatCostUsd(usage.costUsd, i18n.language)}
            </span>
          )}
        </span>
      }
      actions={
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            useNavigationStore
              .getState()
              .navigate({ kind: 'request', requestId: task.id })
          }
        >
          {t('request.openAsRequest')}
        </Button>
      }
    />
  )
}

function Bubble({
  icon,
  title,
  at,
  children,
  tone = 'default',
  testId,
}: {
  icon: React.ReactNode
  title: string
  at: number
  children?: React.ReactNode
  tone?: 'default' | 'muted' | 'warning' | 'error'
  testId?: string
}) {
  const { i18n } = useTranslation()
  const toneClass = {
    default: 'bg-card',
    muted: 'bg-muted/40',
    warning: 'border-warning/40 bg-warning/5',
    error: 'border-destructive/40 bg-destructive/5',
  }[tone]
  return (
    <div
      className={`rounded-lg border p-3 text-sm ${toneClass}`}
      data-testid={testId}
    >
      <div className="flex items-center gap-2">
        <span className="shrink-0 text-muted-foreground [&_svg]:size-4">
          {icon}
        </span>
        <span className="min-w-0 flex-1 font-medium">{title}</span>
        <time
          className="shrink-0 text-xs tabular-nums text-muted-foreground"
          dateTime={new Date(at).toISOString()}
        >
          {new Date(at).toLocaleTimeString(i18n.language)}
        </time>
      </div>
      {children && <div className="mt-2 space-y-1">{children}</div>}
    </div>
  )
}

function JsonBlock({ value }: { value: JsonValue | null }) {
  if (value === null) return null
  const text = typeof value === 'string' ? value : formatJson(value)
  return (
    <pre
      dir="ltr"
      className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md bg-muted/60 p-2 font-mono text-xs"
    >
      {text}
    </pre>
  )
}

interface QuestionPayload {
  id?: string
  question?: string
  topic?: string
  requiresHuman?: boolean
}

/** The question texts asked so far, by question ID */
function questionTexts(thread: ThreadItem[]): Map<string, string> {
  const texts = new Map<string, string>()
  for (const item of thread) {
    if (item.kind !== 'message') continue
    if (item.message.kind !== 'clarification_request') continue
    const questions = asRecord(item.message.payload)?.questions
    if (!Array.isArray(questions)) continue
    for (const q of questions as QuestionPayload[]) {
      if (q?.id && q.question) texts.set(q.id, q.question)
    }
  }
  return texts
}

function ThreadEntry({
  item,
  thread,
}: {
  item: ThreadItem
  thread: ThreadItem[]
}) {
  const { t } = useTranslation()

  switch (item.kind) {
    case 'delegate':
      return (
        <Bubble
          icon={<Send />}
          title={t('delegationThread.item.delegated', {
            requester: isUserRequester(item.task.requesterAgent)
              ? t('tasks.you')
              : item.task.requesterAgent,
            provider: item.task.providerAgent,
            capability: item.task.capability,
          })}
          at={item.at}
          testId="thread-delegate"
        >
          <JsonBlock value={item.task.input} />
        </Bubble>
      )
    case 'state':
      return (
        <div
          className="flex items-center gap-2 px-1 text-xs text-muted-foreground"
          data-testid="thread-state"
        >
          <ArrowRightLeft className="size-3.5 shrink-0" />
          <span>
            {item.event.fromState
              ? t('delegationThread.item.stateChange', {
                  from: t(`tasks.state.${item.event.fromState}`),
                  to: t(`tasks.state.${item.event.toState}`),
                })
              : t('delegationThread.item.stateInitial', {
                  to: t(`tasks.state.${item.event.toState}`),
                })}
            {item.event.reason ? ` — ${item.event.reason}` : ''}
          </span>
        </div>
      )
    case 'message':
      return <MessageEntry message={item.message} thread={thread} />
    case 'result': {
      const failed = item.task.state !== 'completed'
      return (
        <Bubble
          icon={<Flag />}
          title={t(
            `delegationThread.item.result.${failed ? 'failed' : 'completed'}`,
            {
              state: t(`tasks.state.${item.task.state}`),
            }
          )}
          at={item.at}
          tone={failed ? 'error' : 'default'}
          testId="thread-result"
        >
          {item.task.error && (
            <p className="text-sm text-destructive">{item.task.error}</p>
          )}
          <JsonBlock value={item.task.result} />
        </Bubble>
      )
    }
  }
}

function MessageEntry({
  message,
  thread,
}: {
  message: TaskMessage
  thread: ThreadItem[]
}) {
  const { t } = useTranslation()
  const payload = asRecord(message.payload)

  switch (message.kind) {
    case 'progress': {
      const percent = asNumber(payload?.percent)
      const note = typeof payload?.note === 'string' ? payload.note : null
      return (
        <Bubble
          icon={<Activity />}
          title={
            percent !== null
              ? t('delegationThread.item.progressPercent', { percent })
              : t('delegationThread.item.progress')
          }
          at={message.at}
          tone="muted"
          testId="thread-progress"
        >
          {note && <p>{note}</p>}
        </Bubble>
      )
    }
    case 'status_query':
      return (
        <Bubble
          icon={<CircleHelp />}
          title={t('delegationThread.item.statusQuery')}
          at={message.at}
          tone="muted"
          testId="thread-status-query"
        />
      )
    case 'status_response': {
      const state = typeof payload?.state === 'string' ? payload.state : null
      const eta = asNumber(payload?.etaSeconds)
      return (
        <Bubble
          icon={<Activity />}
          title={t('delegationThread.item.statusResponse', {
            state: state ? t(`tasks.state.${state}`) : '—',
          })}
          at={message.at}
          tone="muted"
          testId="thread-status-response"
        >
          {eta !== null && (
            <p>{t('tasks.panel.eta', { eta: formatDuration(eta * 1000) })}</p>
          )}
        </Bubble>
      )
    }
    case 'clarification_request': {
      const questions = Array.isArray(payload?.questions)
        ? (payload.questions as QuestionPayload[])
        : []
      return (
        <Bubble
          icon={<CircleHelp />}
          title={t('delegationThread.item.clarificationRequest', {
            round: asNumber(payload?.round) ?? 1,
          })}
          at={message.at}
          tone="warning"
          testId="thread-clarification-request"
        >
          <ul className="list-inside list-disc">
            {questions.map((q, index) => (
              <li key={q.id ?? index}>
                {q.question}
                {q.topic && (
                  <span className="ms-1 text-xs text-muted-foreground">
                    ({q.topic})
                  </span>
                )}
              </li>
            ))}
          </ul>
        </Bubble>
      )
    }
    case 'clarification_response': {
      const answers = asRecord(payload?.answers ?? null) ?? {}
      const texts = questionTexts(thread)
      return (
        <Bubble
          icon={<MessageSquareReply />}
          title={t('delegationThread.item.clarificationResponse', {
            round: asNumber(payload?.round) ?? 1,
          })}
          at={message.at}
          testId="thread-clarification-response"
        >
          <dl className="space-y-1">
            {Object.entries(answers).map(([questionId, answer]) => (
              <div key={questionId}>
                <dt className="text-xs text-muted-foreground">
                  {texts.get(questionId) ?? questionId}
                </dt>
                <dd>{String(answer)}</dd>
              </div>
            ))}
          </dl>
        </Bubble>
      )
    }
    case 'escalation':
    case 'clarification_escalated':
    case 'clarification_timeout': {
      const question =
        typeof payload?.question === 'string'
          ? payload.question
          : typeof asRecord(payload?.question ?? null)?.question === 'string'
            ? String(asRecord(payload?.question ?? null)?.question)
            : null
      return (
        <Bubble
          icon={
            message.kind === 'clarification_timeout' ? (
              <ShieldAlert />
            ) : (
              <UserRound />
            )
          }
          title={t(
            message.kind === 'clarification_timeout'
              ? 'delegationThread.item.clarificationTimeout'
              : 'delegationThread.item.escalation'
          )}
          at={message.at}
          tone="warning"
          testId="thread-escalation"
        >
          {question && <p>{question}</p>}
        </Bubble>
      )
    }
    case 'reassigned': {
      const text = (key: string) =>
        typeof payload?.[key] === 'string' ? (payload[key] as string) : null
      const to = text('toName') ?? text('toInstance') ?? '—'
      const by = text('by') ?? '—'
      const reason = text('reason')
      return (
        <Bubble
          icon={<ArrowRightLeft />}
          title={t('delegationThread.item.reassigned', { to, by })}
          at={message.at}
          tone="muted"
          testId="thread-reassigned"
        >
          {reason && <p>{reason}</p>}
        </Bubble>
      )
    }
    case 'failed_over': {
      // The relay gave it to the next candidate (relay design step 11)
      const text = (key: string) =>
        typeof payload?.[key] === 'string' ? (payload[key] as string) : null
      const to = text('toName') ?? text('to') ?? '—'
      const cause = text('cause') ?? 'rejected'
      return (
        <Bubble
          icon={<ArrowRightLeft />}
          title={t('delegationThread.item.failedOver', { to })}
          at={message.at}
          tone="muted"
          testId="thread-failed-over"
        >
          <p>{t(`delegationThread.item.failedOverCause.${cause}`)}</p>
        </Bubble>
      )
    }
    case 'route': {
      // Why the relay picked this provider (its typed reasons)
      const provider =
        typeof payload?.provider === 'string' ? payload.provider : '—'
      return (
        <Bubble
          icon={<Waypoints />}
          title={t('delegationThread.item.route', { name: provider })}
          at={message.at}
          tone="muted"
          testId="thread-route"
        >
          <RouteReasons reasons={routeReasonsOf(message.payload)} />
        </Bubble>
      )
    }
    case 'note': {
      const text =
        typeof message.payload === 'string'
          ? message.payload
          : typeof payload?.text === 'string'
            ? payload.text
            : null
      return (
        <Bubble
          icon={<StickyNote />}
          title={t('delegationThread.item.note')}
          at={message.at}
          tone="muted"
          testId="thread-note"
        >
          {text ? <p>{text}</p> : <JsonBlock value={message.payload} />}
        </Bubble>
      )
    }
    default:
      return (
        <Bubble
          icon={<StickyNote />}
          title={t('delegationThread.item.other', { kind: message.kind })}
          at={message.at}
          tone="muted"
          testId="thread-other"
        >
          <JsonBlock value={message.payload} />
        </Bubble>
      )
  }
}

/** A clarification waiting on an answer here (the clarification cards) */
function PendingCard({
  clarification,
}: {
  clarification: PendingClarification
}) {
  // The user's own delegation has no agent to answer: it's always theirs
  const escalated =
    clarification.approvalId !== null ||
    clarification.question.requiresHuman ||
    clarification.agentId === USER_REQUESTER
  return escalated ? (
    <EscalationApprovalCard clarification={clarification} />
  ) : (
    <ClarificationCard clarification={clarification} />
  )
}

function ThreadSidePanel({ task }: { task: TaskRecord }) {
  const { t, i18n } = useTranslation()
  const agentLabel = useAgentLabel()
  const canRerun = isLocalAgent(task.requesterAgent)
  const byUser = isUserRequester(task.requesterAgent)
  // The requester's own policy, when it's an agent here
  const policy = useClarificationPolicy(task.requesterAgent, canRerun)
  const knownPolicy = policy.isSuccess ? policy.data : null

  const rerun = () => {
    // Its question policy lives in the agent's Settings › Sharing
    useNavigationStore.getState().navigate({
      kind: 'agent',
      agentId: task.requesterAgent,
      tab: 'settings',
    })
  }

  const facts: [string, string][] = [
    [
      t('delegationThread.side.source'),
      t(`delegationThread.source.${task.sourceKind}`),
    ],
    [t('delegationThread.side.attempt'), String(task.attemptNumber)],
    [t('delegationThread.side.priority'), String(task.priority)],
    [
      t('delegationThread.side.created'),
      new Date(task.createdAt).toLocaleString(i18n.language),
    ],
  ]
  if (task.etaSeconds !== null) {
    facts.push([
      t('delegationThread.side.eta'),
      formatDuration(task.etaSeconds * 1000),
    ])
  }
  if (task.deadline !== null) {
    facts.push([
      t('delegationThread.side.deadline'),
      new Date(task.deadline).toLocaleString(i18n.language),
    ])
  }

  return (
    <aside
      className="hidden w-72 shrink-0 flex-col gap-5 overflow-y-auto border-s p-4 text-sm lg:flex"
      aria-label={t('delegationThread.side.label')}
    >
      <section className="space-y-2">
        <h2 className="text-xs font-medium uppercase text-muted-foreground">
          {t('delegationThread.side.policy')}
        </h2>
        <p data-testid="delegation-thread-policy">
          {summarizePolicy(knownPolicy ?? DEFAULT_CLARIFICATION_POLICY, t)}
        </p>
        <p className="text-xs text-muted-foreground">
          {byUser
            ? t('delegationThread.side.policyUser')
            : knownPolicy
              ? t('delegationThread.side.policyOwn')
              : t('delegationThread.side.policyNote')}
        </p>
        <Button
          variant="outline"
          size="sm"
          disabled={!canRerun}
          title={
            canRerun
              ? undefined
              : t(
                  byUser
                    ? 'delegationThread.side.rerunUser'
                    : 'delegationThread.side.rerunRemote'
                )
          }
          onClick={rerun}
        >
          {t('delegationThread.side.rerun')}
        </Button>
      </section>

      <section className="space-y-2">
        <h2 className="text-xs font-medium uppercase text-muted-foreground">
          {t('delegationThread.side.details')}
        </h2>
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1">
          {facts.map(([label, value]) => (
            <div key={label} className="contents">
              <dt className="text-muted-foreground">{label}</dt>
              <dd className="min-w-0 truncate">{value}</dd>
            </div>
          ))}
        </dl>
        {task.previousAttemptId && (
          <Button
            variant="link"
            size="sm"
            className="h-auto p-0"
            onClick={() =>
              useNavigationStore.getState().navigate({
                kind: 'delegation-thread',
                taskId: task.previousAttemptId ?? '',
              })
            }
          >
            {t('delegationThread.side.previousAttempt')}
          </Button>
        )}
      </section>

      <section className="space-y-2">
        <h2 className="text-xs font-medium uppercase text-muted-foreground">
          {t('delegationThread.side.provider')}
        </h2>
        <p className="truncate" dir="ltr">
          {agentLabel(task.providerAgent, task.providerInstance)}
        </p>
        <Button
          variant="outline"
          size="sm"
          disabled={!task.providerInstance}
          onClick={() =>
            useNavigationStore.getState().navigate({
              kind: 'member-profile',
              instanceId: task.providerInstance,
            })
          }
        >
          <UserRound />
          {t('delegationThread.side.providerProfile')}
        </Button>
      </section>
    </aside>
  )
}
