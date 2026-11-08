import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Sparkles } from 'lucide-react'
import { MessageList } from '@/components/agent-console/MessageList'
import { isWorking } from '@/components/agent-console/working'
import { AskBox } from '@/components/ask/AskBox'
import { useNow } from '@/components/agents/task-utils'
import {
  AlwaysAllowDialog,
  ApproveSuspiciousDialog,
  QueueRowView,
} from '@/components/home/QueueRowView'
import { RequestSteps } from '@/components/requests/RequestSteps'
import { SaveAsWorkflowDialog } from '@/components/requests/SaveAsWorkflowDialog'
import { localStepPosition } from '@/components/requests/request-model'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { useAlwaysAllow } from '@/hooks/useAutoApproveRules'
import { errorMessage } from '@/hooks/useDelegation'
import { useIntakeThread } from '@/hooks/useIntakeThread'
import { useQueueDecisions } from '@/hooks/useQueueDecisions'
import { useCompleteRequestStep, useRequest } from '@/hooks/useRequests'
import { decideApproval, sendIntakeMessage } from '@/lib/agent-actions'
import { CHIEF_OF_STAFF_AGENT_ID, displayAgent } from '@/lib/agent-identity'
import { loadConversation } from '@/lib/agent-sync'
import { askContextForView, openEditInStudio } from '@/lib/ask'
import type { Request, WorkflowProposal } from '@/lib/bindings'
import { useConversation } from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'
import {
  canMarkStepsDone,
  threadModel,
  workflowProposalFor,
} from './intake-thread-model'
import { ViewHeader } from './ViewHeader'

/**
 * One intake item's conversation with the Chief of Staff. Above: what waits
 * for the user's OK (Approve or Dismiss), what was decided, and the Requests
 * it became with their steps. Below: the conversation, where the user can
 * ask for a draft or hand the work to an agent (which queues it here for
 * approval).
 */
export function IntakeThreadView({
  conversationId,
}: {
  conversationId: string
}) {
  const { t, i18n } = useTranslation()
  const thread = useIntakeThread(conversationId)
  const decisions = useQueueDecisions('request')
  const alwaysAllow = useAlwaysAllow()

  if (thread.isPending) {
    return (
      <div className="flex flex-col gap-3 p-4">
        <Skeleton className="h-6 w-1/3" />
        <Skeleton className="h-24 w-full" />
      </div>
    )
  }
  if (thread.isError) {
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyTitle>{t('intakeThread.loadError.title')}</EmptyTitle>
          <EmptyDescription>{errorMessage(thread.error)}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  const model = threadModel(thread.data, i18n.language)
  const hasTop =
    model.pending.length > 0 ||
    model.decided.length > 0 ||
    thread.data.requestIds.length > 0

  return (
    <section
      aria-label={t('intakeThread.label')}
      className="flex h-full min-h-0 flex-col"
    >
      <ViewHeader
        leading={
          <Sparkles aria-hidden className="size-4 text-muted-foreground" />
        }
        title={thread.data.title}
        subtitle={t(model.sourceKey)}
      />
      {hasTop && (
        <div
          data-testid="intake-thread-top"
          className="flex max-h-[45%] shrink-0 flex-col gap-3 overflow-y-auto border-b p-4"
        >
          {model.pending.map(row => (
            <QueueRowView
              key={row.id}
              row={row}
              clamp={false}
              busy={decisions.busy || alwaysAllow.busy}
              onApprove={() => decisions.requestApprove(row)}
              onDismiss={() => decisions.dismiss(row)}
              onAlwaysAllow={() => alwaysAllow.ask(row)}
            />
          ))}
          {model.decided.length > 0 && (
            <ul
              aria-label={t('intakeThread.history')}
              className="flex flex-col gap-1 text-xs text-muted-foreground"
            >
              {model.decided.map(row => (
                <li key={row.id} className="flex min-w-0 items-center gap-2">
                  <Badge variant="outline" className="shrink-0">
                    {t(row.statusKey)}
                  </Badge>
                  <span className="min-w-0 flex-1 truncate">{row.text}</span>
                  <span className="shrink-0">{row.when}</span>
                </li>
              ))}
            </ul>
          )}
          {thread.data.requestIds.map(requestId => (
            <ThreadRequest
              key={requestId}
              requestId={requestId}
              proposal={workflowProposalFor(thread.data, requestId)}
            />
          ))}
        </div>
      )}
      <ThreadConversation conversationId={conversationId} />
      <div className="shrink-0 border-t bg-background p-3">
        <ThreadComposer conversationId={conversationId} />
      </div>
      <ApproveSuspiciousDialog
        open={decisions.confirming !== null}
        onCancel={decisions.cancelConfirm}
        onApprove={decisions.approveConfirmed}
      />
      <AlwaysAllowDialog
        domain={alwaysAllow.asking?.alwaysAllowDomain ?? null}
        onCancel={alwaysAllow.cancel}
        onConfirm={alwaysAllow.confirm}
      />
    </section>
  )
}

/**
 * A Request the thread became: its status and steps, "Mark done" included,
 * and "Save as workflow" when the Chief of Staff proposes it
 */
function ThreadRequest({
  requestId,
  proposal,
}: {
  requestId: string
  proposal: WorkflowProposal | null
}) {
  const request = useRequest(requestId)
  // A Request that can't be read (e.g. a teammate's task gone) is left out
  if (!request.data) return null
  return <RequestCard request={request.data} proposal={proposal} />
}

function RequestCard({
  request,
  proposal,
}: {
  request: Request
  proposal: WorkflowProposal | null
}) {
  const { t } = useTranslation()
  const now = useNow(request.endedAt === null)
  const completeStep = useCompleteRequestStep(request.id)
  const local = canMarkStepsDone(request)
  const [saving, setSaving] = useState(false)

  return (
    <div
      data-testid="intake-thread-request"
      className="flex min-w-0 flex-col gap-2 rounded-md border px-3 py-2"
    >
      <div className="flex min-w-0 items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {request.title}
        </span>
        <Badge variant="secondary" className="shrink-0">
          {t(`request.status.${request.status}`)}
        </Badge>
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            useNavigationStore
              .getState()
              .navigate({ kind: 'request', requestId: request.id })
          }
        >
          {t('intakeThread.openRequest')}
        </Button>
      </div>
      {proposal && (
        <div
          data-testid="intake-thread-workflow-proposal"
          className="flex min-w-0 items-center gap-2 text-sm"
        >
          <span className="min-w-0 flex-1 text-muted-foreground">
            {t('request.saveWorkflow.proposal', { title: proposal.likeTitle })}
          </span>
          <Button size="sm" onClick={() => setSaving(true)}>
            {t('request.saveWorkflow.button')}
          </Button>
          <SaveAsWorkflowDialog
            requestId={request.id}
            open={saving}
            onOpenChange={setSaving}
          />
        </div>
      )}
      <RequestSteps
        request={request}
        now={now}
        focused={null}
        onComplete={
          local
            ? step => {
                const position = localStepPosition(step)
                if (position === null) return
                completeStep.mutate(position, {
                  onError: error => toast.error(errorMessage(error)),
                })
              }
            : undefined
        }
      />
    </div>
  )
}

function ThreadConversation({ conversationId }: { conversationId: string }) {
  const { t } = useTranslation()
  const conversation = useConversation(conversationId)
  const assistant = displayAgent(
    CHIEF_OF_STAFF_AGENT_ID,
    t('intakeThread.assistant')
  )

  useEffect(() => {
    if (!conversation) void loadConversation(conversationId)
  }, [conversationId, conversation])

  if (!conversation) {
    return (
      <div className="flex flex-1 items-center justify-center">
        <Spinner />
      </div>
    )
  }
  return (
    <MessageList
      messages={conversation.messages}
      assistant={assistant}
      working={isWorking(conversation.currentRun, conversation.messages)}
      onDecide={(approvalId, decision) =>
        decideApproval(CHIEF_OF_STAFF_AGENT_ID, approvalId, decision)
      }
    />
  )
}

function ThreadComposer({ conversationId }: { conversationId: string }) {
  const { t } = useTranslation()
  const view = useNavigationStore(state => state.current)
  return (
    <AskBox
      className="mx-auto max-w-3xl"
      context={askContextForView(view)}
      placeholder={t('intakeThread.placeholder')}
      onApplyEdit={openEditInStudio}
      fixedMode="chat"
      chat={{
        placeholder: t('intakeThread.placeholder'),
        onSend: text => void sendIntakeMessage(conversationId, text),
      }}
    />
  )
}
