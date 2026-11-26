import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  ArrowLeft,
  History,
  Radio,
  RotateCcw,
  TriangleAlert,
} from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyContent,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { AgentAvatar } from '@/components/agent-console/AgentAvatar'
import { MessageList } from '@/components/agent-console/MessageList'
import { StopRunButton } from '@/components/agent-console/StopRunButton'
import { isWorking } from '@/components/agent-console/working'
import { AskBox } from '@/components/ask/AskBox'
import { ViewHeader } from '@/components/views/ViewHeader'
import { useAgents } from '@/hooks/useAgents'
import { useRunPlan } from '@/hooks/useRunPlan'
import { useRunDetail } from '@/hooks/useSnapshots'
import { useRetryRun } from '@/hooks/useWork'
import { decideApproval, replyToRun } from '@/lib/agent-actions'
import { displayAgent } from '@/lib/agent-identity'
import { loadRunConversation } from '@/lib/agent-sync'
import { askContextForView, openEditInStudio } from '@/lib/ask'
import { RUN_STATUS_CLASSES } from '@/lib/run-status'
import type { RunDetail, RunStatus, RunSummary } from '@/lib/tauri-bindings'
import { useAgentQueue, useConversation } from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'
import {
  isLiveStatus,
  labelText,
  liveOrigin,
  messagesFrom,
  statusLine,
  statusText,
  waitingReplies,
} from './live-run-model'

/**
 * One live run as a chat, however it started: its messages as they
 * stream, its plan card, its approval cards (decided right here, through
 * the same `decideApproval` as everywhere else) and Stop. Run Detail keeps
 * its history.
 */
export function LiveRunView({ runId }: { runId: string }) {
  const { t } = useTranslation()
  const { data: detail, isLoading } = useRunDetail(runId)

  if (isLoading) {
    return (
      <div className="flex flex-col gap-3 p-4" aria-busy="true">
        <Skeleton className="h-10 w-full" />
        <Skeleton className="h-24 w-full" />
      </div>
    )
  }
  if (!detail) {
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <Radio />
          </EmptyMedia>
          <EmptyTitle>{t('liveWork.gone')}</EmptyTitle>
        </EmptyHeader>
        <EmptyContent>
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              useNavigationStore.getState().navigate({ kind: 'nandi' })
            }
          >
            {t('liveWork.backToNandi')}
          </Button>
        </EmptyContent>
      </Empty>
    )
  }
  return <LiveChat detail={detail} />
}

function LiveChat({ detail }: { detail: RunDetail }) {
  const { t } = useTranslation()
  const { run } = detail
  const conversation = useConversation(run.conversationId)
  const agents = useAgents().data ?? []
  const agent =
    agents.find(a => a.id === run.agentId) ??
    displayAgent(run.agentId, detail.agentName)
  const origin = liveOrigin(run, id => agents.find(a => a.id === id)?.name)

  // Its conversation in the store, so its events stream in
  useEffect(() => {
    if (!conversation) void loadRunConversation(run.id, run.conversationId)
  }, [run.id, run.conversationId, conversation])

  const live = conversation ? messagesFrom(conversation.messages, run.id) : []
  const messages = live.length > 0 ? live : detail.messages
  // The turn going now: this run, or a later one in the same chat
  const current = conversation
    ? conversation.currentRun
    : isLiveStatus(run.status)
      ? run
      : null
  const shown = current ?? run
  const { data: plan } = useRunPlan(shown.id)
  const line = current ? statusLine(plan, messages) : null

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        leading={
          <div className="flex items-center gap-2">
            <Button
              variant="ghost"
              size="icon"
              className="size-7"
              aria-label={t('views.back')}
              onClick={() => useNavigationStore.getState().goBack()}
            >
              <ArrowLeft className="rtl:rotate-180" />
            </Button>
            <AgentAvatar agent={agent} className="size-8" />
          </div>
        }
        title={agent.name}
        titleAdornment={<LiveBadge status={shown.status} />}
        subtitle={
          <span data-testid="live-status-line">
            {line ? statusText(line, t) : labelText(origin, t)}
          </span>
        }
        actions={
          <>
            {/* Stop is one tap, at the top, while it works */}
            {current && <LiveStop runId={current.id} />}
            <Button
              size="sm"
              variant="ghost"
              onClick={() =>
                useNavigationStore
                  .getState()
                  .navigate({ kind: 'run-detail', runId: shown.id })
              }
            >
              <History />
              {t('liveWork.details')}
            </Button>
          </>
        }
      />
      {!current && shown.status === 'failed' && (
        <FailedLine runId={shown.id} summary={shown.summary} />
      )}
      <MessageList
        messages={messages}
        assistant={agent}
        working={isWorking(current, messages)}
        quietSteps
        onDecide={(approvalId, decision) =>
          decideApproval(run.agentId, approvalId, decision)
        }
        empty={
          <div className="flex h-full items-center justify-center">
            <Spinner />
          </div>
        }
      />
      <LiveReply
        run={run}
        agentName={agent.name}
        failed={!current && shown.status === 'failed'}
      />
    </div>
  )
}

/**
 * Steer while it works: a reply is the next turn in the run's own
 * conversation, as its agent. Sent while the agent works, it waits in the
 * agent's queue and shows as "<Agent> will read this next".
 */
function LiveReply({
  run,
  agentName,
  failed,
}: {
  run: RunSummary
  agentName: string
  failed: boolean
}) {
  const { t } = useTranslation()
  const view = useNavigationStore(state => state.current)
  const queue = useAgentQueue(run.agentId)
  const [sent, setSent] = useState<string[]>([])
  const waiting = waitingReplies(queue, sent)
  const placeholder = failed
    ? t('liveWork.reply.whatToChange', { name: agentName })
    : t('liveWork.reply.placeholder', { name: agentName })

  return (
    <div className="shrink-0 border-t p-3">
      {waiting.map(entry => (
        <p
          key={entry.id}
          data-testid="live-reply-next"
          className="mx-auto max-w-3xl truncate pb-2 text-xs text-muted-foreground"
        >
          {t('liveWork.reply.next', { name: agentName, text: entry.message })}
        </p>
      ))}
      <AskBox
        className="mx-auto max-w-3xl"
        context={askContextForView(view)}
        placeholder={placeholder}
        onApplyEdit={openEditInStudio}
        fixedMode="chat"
        chat={{
          placeholder,
          onSend: (text, attachments) =>
            void replyToRun(run.id, run.conversationId, text, attachments).then(
              result => {
                if (result?.queued) setSent(ids => [...ids, result.runId])
              }
            ),
        }}
      />
    </div>
  )
}

/** "Needs you" when it waits on the person, else its status */
export function LiveBadge({ status }: { status: RunStatus }) {
  const { t } = useTranslation()
  return (
    <Badge variant="outline" className={RUN_STATUS_CLASSES[status]}>
      {status === 'awaitingApproval'
        ? t('liveWork.needsYou')
        : t(`runs.status.${status}`)}
    </Badge>
  )
}

/** Calm failure: what happened in one line, and Try again */
function FailedLine({
  runId,
  summary,
}: {
  runId: string
  summary: string | null
}) {
  const { t } = useTranslation()
  const retry = useRetryRun()
  const what = summary?.trim().split('\n')[0]
  return (
    <div
      role="status"
      className="flex flex-wrap items-center gap-2 border-b bg-destructive/5 px-4 py-2 text-sm"
    >
      <TriangleAlert aria-hidden className="size-4 shrink-0 text-destructive" />
      <span className="min-w-0 flex-1 truncate">
        {what ? t('liveWork.failedWith', { what }) : t('liveWork.failed')}
      </span>
      <Button
        size="sm"
        variant="outline"
        disabled={retry.isPending}
        onClick={() =>
          void retry
            .mutateAsync(runId)
            .then(newRunId =>
              useNavigationStore
                .getState()
                .navigate({ kind: 'live-run', runId: newRunId })
            )
        }
      >
        <RotateCcw />
        {t('liveWork.tryAgain')}
      </Button>
    </div>
  )
}

/** Stop for the turn going now, while it's going */
export function LiveStop({ runId }: { runId: string }) {
  const { data } = useRunDetail(runId)
  if (!data || !isLiveStatus(data.run.status)) return null
  return <StopRunButton detail={data} />
}
