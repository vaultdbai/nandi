import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Check, Clock, ShieldAlert, X } from 'lucide-react'
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
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { useNow } from '@/components/agents/task-utils'
import {
  approvalClock,
  approvalPlan,
  approvalSummary,
  delegateWho,
  minutes,
} from '@/components/inbox/approval-model'
import { useNetworkSnapshot } from '@/hooks/useNetwork'
import { ActionPlanPreview } from '@/components/ask/ActionPlanPreview'
import { cn } from '@/lib/utils'
import type { ChatMessage } from '@/types/agent'
import {
  ACCESS_TOOL,
  agentLabel,
  parseAccessRequest,
} from '@/components/connectors/access-model'
import { AccessRequestCard } from '@/components/connectors/AccessRequestCard'
import { formatJson } from './format-json'

type ApprovalRequestMessage = Extract<ChatMessage, { kind: 'approval' }>

/** What the card needs: a chat approval message, or a pending approval */
export type ApprovalCardData = Pick<
  ApprovalRequestMessage,
  'id' | 'tool' | 'args' | 'status' | 'createdAt' | 'expiresAt'
>

interface ApprovalRequestCardProps {
  message: ApprovalCardData
  /** Shown above the sentence: who's asking (the Inbox, a Request) */
  agentName?: string
  /**
   * Resolves to whether Rust accepted the decision. Omit for a read-only
   * transcript: the card then shows no Approve / Reject
   */
  onDecide?: (
    approvalId: string,
    decision: 'approved' | 'rejected'
  ) => Promise<boolean>
  className?: string
}

/**
 * One tool approval: what it will do in plain words, how long it has left,
 * the raw arguments behind Details, and Approve / Reject. Reject asks first;
 * Approve doesn't (the card itself shows what will happen). The same card
 * is used in chat, Run Detail, the Inbox and a Request.
 */
export function ApprovalRequestCard(props: ApprovalRequestCardProps) {
  const { message } = props
  const access =
    message.tool === ACCESS_TOOL ? parseAccessRequest(message.args) : null
  if (access) {
    return (
      <AccessRequestCard
        approvalId={message.id}
        request={access}
        status={message.status}
        createdAt={message.createdAt}
        expiresAt={message.expiresAt}
        agentName={props.agentName}
        interactive={props.onDecide !== undefined}
        className={props.className}
      />
    )
  }
  return <ToolApprovalCard {...props} />
}

function ToolApprovalCard({
  message,
  agentName,
  onDecide,
  className,
}: ApprovalRequestCardProps) {
  const { t } = useTranslation()
  // Prevents double-submits while the decision round-trips to Rust
  const [submitting, setSubmitting] = useState(false)
  const [confirmingReject, setConfirmingReject] = useState(false)
  const pending = message.status === 'pending'
  const now = useNow(pending, 30_000)
  const summary = approvalSummary(message.tool, message.args)
  // A request to someone on the network: say who, by name
  const network = useNetworkSnapshot(summary.key === 'delegateTo')
  if (summary.key === 'delegateTo') {
    const members = network.data?.members ?? []
    summary.values.who = delegateWho(
      summary.values,
      instance => members.find(m => m.instanceId === instance)?.displayName,
      agentLabel
    )
  }
  // Studio starting a Request: the steps it would take
  const plan = approvalPlan(message.tool, message.args)
  const clock = approvalClock(now, message.createdAt, message.expiresAt)

  // The status changes when Rust's approval_resolved event arrives; until
  // then, block double submits (and re-enable if the command failed)
  const decide = async (decision: 'approved' | 'rejected') => {
    if (!onDecide) return
    setSubmitting(true)
    if (!(await onDecide(message.id, decision))) setSubmitting(false)
  }

  return (
    <Card
      data-testid="approval-card"
      className={cn(
        'gap-3 px-4 py-4 shadow-none',
        pending && 'border-warning/40 bg-warning/5',
        className
      )}
    >
      <div className="flex items-start gap-3">
        <ShieldAlert
          aria-hidden
          className={cn(
            'mt-0.5 size-4 shrink-0',
            pending ? 'text-warning' : 'text-muted-foreground'
          )}
        />
        <div className="min-w-0 flex-1 space-y-1">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {agentName
              ? t('approval.labelFrom', { agent: agentName })
              : t('approval.label')}
          </p>
          <p className="text-sm font-medium break-words text-foreground">
            {t(`approval.summary.${summary.key}`, summary.values)}
          </p>
          {pending && (
            <p className="flex items-center gap-1 text-xs text-muted-foreground">
              <Clock aria-hidden className="size-3" />
              {clock.kind === 'expiresIn'
                ? t('approval.expiresIn', { minutes: minutes(clock.ms) })
                : clock.kind === 'waiting'
                  ? t('approval.waiting', { minutes: minutes(clock.ms) })
                  : t('approval.expiring')}
            </p>
          )}
        </div>
        {message.status !== 'pending' && (
          <ApprovalStatusBadge status={message.status} />
        )}
      </div>

      {plan && (
        <section aria-label={t('approval.plan')}>
          <ActionPlanPreview plan={plan} />
        </section>
      )}

      <Collapsible>
        <CollapsibleTrigger className="text-xs text-muted-foreground underline-offset-2 hover:underline">
          {t('approval.details')}
        </CollapsibleTrigger>
        <CollapsibleContent>
          <pre
            dir="ltr"
            className="mt-2 max-h-64 overflow-auto rounded-md border bg-muted/60 px-3 py-2 text-start font-mono text-xs whitespace-pre-wrap break-all text-foreground"
          >
            {`${message.tool}\n${formatJson(message.args)}`}
          </pre>
        </CollapsibleContent>
      </Collapsible>

      {pending && onDecide && (
        <div className="flex justify-end gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={submitting}
            onClick={() => setConfirmingReject(true)}
          >
            <X />
            {t('approval.reject')}
          </Button>
          <Button
            size="sm"
            disabled={submitting}
            onClick={() => void decide('approved')}
          >
            <Check />
            {t('approval.approve')}
          </Button>
        </div>
      )}

      <AlertDialog open={confirmingReject} onOpenChange={setConfirmingReject}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('approval.rejectConfirm.title')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('approval.rejectConfirm.description')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('approval.rejectConfirm.keep')}
            </AlertDialogCancel>
            <AlertDialogAction onClick={() => void decide('rejected')}>
              {t('approval.reject')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  )
}

function ApprovalStatusBadge({
  status,
}: {
  status: Exclude<ApprovalRequestMessage['status'], 'pending'>
}) {
  const { t } = useTranslation()

  switch (status) {
    case 'approved':
      return (
        <Badge
          variant="outline"
          className="border-success/30 bg-success/10 text-success"
        >
          <Check />
          {t('agentConsole.approval.approved')}
        </Badge>
      )
    case 'rejected':
      return (
        <Badge variant="outline" className="text-muted-foreground">
          <X />
          {t('agentConsole.approval.rejected')}
        </Badge>
      )
    case 'expired':
      return (
        <Badge variant="outline" className="text-muted-foreground">
          <Clock />
          {t('agentConsole.approval.expired')}
        </Badge>
      )
  }
}
