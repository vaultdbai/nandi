import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Check, Clock, Hand, PencilLine, SkipForward, X } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Textarea } from '@/components/ui/textarea'
import { useNow } from '@/components/agents/task-utils'
import { approvalClock, handOverDraft } from '@/components/inbox/approval-model'
import { answerHandOver } from '@/lib/agent-actions'
import type { HandOverOutcome } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import type { ApprovalCardData } from './ApprovalRequestCard'

interface HandOverCardProps {
  message: ApprovalCardData
  agentName?: string
  /** Present when the person may answer (not a read-only transcript) */
  canAnswer: boolean
  className?: string
}

/**
 * A browser hand-over (`pilots::handover`): the agent prepared a page in
 * its browser window, and the person finishes it there. They press Post
 * themselves, then say what happened: Done, Skip, or Changed it with the
 * text that went out. The same card shows in chat, Run Detail, the Inbox
 * and a Request.
 */
export function HandOverCard({
  message,
  agentName,
  canAnswer,
  className,
}: HandOverCardProps) {
  const { t } = useTranslation()
  const [submitting, setSubmitting] = useState(false)
  const [editing, setEditing] = useState(false)
  const { site, summary, draft } = handOverDraft(message.args)
  const [finalText, setFinalText] = useState(draft)
  const pending = message.status === 'pending'
  const now = useNow(pending, 60_000)
  const clock = approvalClock(now, message.createdAt, message.expiresAt)

  // The card changes when Rust's approval_resolved event arrives; until
  // then, block double submits (and re-enable if the command failed)
  const answer = async (outcome: HandOverOutcome) => {
    setSubmitting(true)
    const ok = await answerHandOver(message.id, {
      outcome,
      finalText: outcome === 'changed' ? finalText : null,
    })
    if (!ok) setSubmitting(false)
  }

  return (
    <Card
      data-testid="hand-over-card"
      className={cn(
        'gap-3 px-4 py-4 shadow-none',
        pending && 'border-info/40 bg-info/5',
        className
      )}
    >
      <div className="flex items-start gap-3">
        <Hand
          aria-hidden
          className={cn(
            'mt-0.5 size-4 shrink-0',
            pending ? 'text-info' : 'text-muted-foreground'
          )}
        />
        <div className="min-w-0 flex-1 space-y-1">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {agentName
              ? t('handOver.labelFrom', { agent: agentName })
              : t('handOver.label')}
          </p>
          <p className="text-sm font-medium break-words text-foreground">
            {summary || t('handOver.untitled')}
            {site && (
              <span className="font-normal text-muted-foreground">
                {' · '}
                {t('handOver.on', { site })}
              </span>
            )}
          </p>
          {pending && clock.kind === 'expiresIn' && (
            <p className="flex items-center gap-1 text-xs text-muted-foreground">
              <Clock aria-hidden className="size-3" />
              {t('handOver.waits', {
                hours: Math.max(1, Math.round(clock.ms / 3_600_000)),
              })}
            </p>
          )}
        </div>
        {!pending && <HandOverStatus status={message.status} />}
      </div>

      {pending && (
        <p className="text-sm text-muted-foreground">{t('handOver.steps')}</p>
      )}

      {draft && !editing && (
        <section aria-label={t('handOver.draft')}>
          <p className="mb-1 text-xs text-muted-foreground">
            {t('handOver.draft')}
          </p>
          <p className="max-h-64 overflow-auto rounded-md border bg-muted/60 px-3 py-2 text-sm whitespace-pre-wrap break-words text-foreground">
            {draft}
          </p>
        </section>
      )}

      {pending && canAnswer && editing && (
        <div className="space-y-2">
          <label
            htmlFor={`hand-over-${message.id}`}
            className="text-xs text-muted-foreground"
          >
            {t('handOver.finalText')}
          </label>
          <Textarea
            id={`hand-over-${message.id}`}
            value={finalText}
            onChange={event => setFinalText(event.target.value)}
            rows={6}
          />
          <div className="flex justify-end gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={submitting}
              onClick={() => setEditing(false)}
            >
              <X />
              {t('handOver.cancel')}
            </Button>
            <Button
              size="sm"
              disabled={submitting || !finalText.trim()}
              onClick={() => void answer('changed')}
            >
              <Check />
              {t('handOver.save')}
            </Button>
          </div>
        </div>
      )}

      {pending && canAnswer && !editing && (
        <div className="flex flex-wrap justify-end gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={submitting}
            onClick={() => void answer('skipped')}
          >
            <SkipForward />
            {t('handOver.skip')}
          </Button>
          <Button
            size="sm"
            variant="outline"
            disabled={submitting}
            onClick={() => setEditing(true)}
          >
            <PencilLine />
            {t('handOver.changed')}
          </Button>
          <Button
            size="sm"
            disabled={submitting}
            onClick={() => void answer('done')}
          >
            <Check />
            {t('handOver.done')}
          </Button>
        </div>
      )}
    </Card>
  )
}

function HandOverStatus({ status }: { status: ApprovalCardData['status'] }) {
  const { t } = useTranslation()
  switch (status) {
    case 'approved':
      return (
        <Badge
          variant="outline"
          className="border-success/30 bg-success/10 text-success"
        >
          <Check />
          {t('handOver.status.answered')}
        </Badge>
      )
    case 'rejected':
      return (
        <Badge variant="outline" className="text-muted-foreground">
          <SkipForward />
          {t('handOver.status.skipped')}
        </Badge>
      )
    case 'expired':
      return (
        <Badge variant="outline" className="text-muted-foreground">
          <Clock />
          {t('handOver.status.expired')}
        </Badge>
      )
    default:
      return null
  }
}
