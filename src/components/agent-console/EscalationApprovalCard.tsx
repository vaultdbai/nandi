import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Check, CheckCheck, Clock, MessageSquareReply, X } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Textarea } from '@/components/ui/textarea'
import type { PendingClarification } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import {
  ALLOW_ALWAYS_ANSWER,
  ALLOW_ONCE_ANSWER,
  DENY_ANSWER,
  USER_REQUESTER,
  answerTo,
  formatRemaining,
  remainingMs,
  useRespondToClarification,
} from '@/hooks/useClarifications'
import { rawError } from '@/lib/friendly-error'

export interface EscalationApprovalCardProps {
  clarification: PendingClarification
  /** The delegated capability, when the caller knows it */
  capability?: string
  /** Who asks (a provider card ID or name) */
  provider?: string
  /** Clock for the countdown (tests) */
  now?: () => number
}

/** Ticks once a second while a deadline is set */
function useNow(enabled: boolean, now: () => number): number {
  const [time, setTime] = useState(now)
  useEffect(() => {
    if (!enabled) return
    const timer = setInterval(() => setTime(now()), 1000)
    return () => clearInterval(timer)
  }, [enabled, now])
  return time
}

/**
 * A provider's question escalated to the user: answer it, let the agent
 * answer it (once, or for its topic from now on), or deny it, which
 * cancels the task. Counts down to the round's deadline.
 */
export function EscalationApprovalCard({
  clarification,
  capability,
  provider,
  now = Date.now,
}: EscalationApprovalCardProps) {
  const { t } = useTranslation()
  const respond = useRespondToClarification()
  const [answering, setAnswering] = useState(false)
  const [draft, setDraft] = useState('')
  const time = useNow(clarification.deadline !== null, now)
  const left = remainingMs(clarification.deadline, time)
  const expired = left === 0
  const { question } = clarification
  // Only an agent can answer on its own
  const agentCanAnswer =
    clarification.agentId !== USER_REQUESTER && !question.requiresHuman
  const busy = respond.isPending || expired

  const send = (answer: string) =>
    respond.mutate(answerTo(clarification, answer))

  return (
    <Card className="gap-3 border-warning/40 bg-warning/5 px-4 py-4 shadow-none">
      <div className="flex items-start gap-3">
        <MessageSquareReply
          aria-hidden
          className="mt-0.5 size-4 shrink-0 text-warning"
        />
        <div className="min-w-0 flex-1 space-y-1">
          <p className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
            {t('clarification.escalation.label')}
          </p>
          <p className="text-sm font-medium text-foreground">
            {question.question}
          </p>
        </div>
        {left !== null && (
          <Badge
            variant="outline"
            className={cn(
              'tabular-nums',
              expired && 'border-destructive/40 text-destructive'
            )}
          >
            <Clock />
            {expired
              ? t('clarification.escalation.expired')
              : t('clarification.escalation.timeLeft', {
                  time: formatRemaining(left),
                })}
          </Badge>
        )}
      </div>

      <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs">
        <dt className="text-muted-foreground">
          {t('clarification.escalation.task')}
        </dt>
        <dd dir="ltr" className="truncate text-start font-mono">
          {clarification.taskId}
        </dd>
        {capability && (
          <>
            <dt className="text-muted-foreground">
              {t('clarification.escalation.capability')}
            </dt>
            <dd className="truncate">{capability}</dd>
          </>
        )}
        {provider && (
          <>
            <dt className="text-muted-foreground">
              {t('clarification.escalation.provider')}
            </dt>
            <dd className="truncate">{provider}</dd>
          </>
        )}
        {question.topic && (
          <>
            <dt className="text-muted-foreground">
              {t('clarification.escalation.topic')}
            </dt>
            <dd className="truncate">{question.topic}</dd>
          </>
        )}
      </dl>

      {answering && (
        <form
          className="space-y-2"
          onSubmit={event => {
            event.preventDefault()
            if (draft.trim()) send(draft.trim())
          }}
        >
          <Textarea
            autoFocus
            value={draft}
            aria-label={t('clarification.escalation.answerLabel')}
            placeholder={t('clarification.escalation.placeholder')}
            onChange={event => setDraft(event.target.value)}
          />
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => setAnswering(false)}
            >
              {t('clarification.escalation.cancel')}
            </Button>
            <Button type="submit" size="sm" disabled={busy || !draft.trim()}>
              {t('clarification.escalation.send')}
            </Button>
          </div>
        </form>
      )}

      {respond.error && (
        <p role="alert" className="text-xs text-destructive">
          {rawError(respond.error)}
        </p>
      )}

      {!answering && (
        <div className="flex flex-wrap justify-end gap-2">
          <Button
            size="sm"
            variant="outline"
            disabled={busy}
            onClick={() => send(DENY_ANSWER)}
          >
            <X />
            {t('clarification.escalation.deny')}
          </Button>
          {agentCanAnswer && (
            <>
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => send(ALLOW_ALWAYS_ANSWER)}
              >
                <CheckCheck />
                {t('clarification.escalation.allowAlways')}
              </Button>
              <Button
                size="sm"
                variant="outline"
                disabled={busy}
                onClick={() => send(ALLOW_ONCE_ANSWER)}
              >
                <Check />
                {t('clarification.escalation.allowOnce')}
              </Button>
            </>
          )}
          <Button size="sm" disabled={busy} onClick={() => setAnswering(true)}>
            <MessageSquareReply />
            {t('clarification.escalation.answer')}
          </Button>
        </div>
      )}
    </Card>
  )
}
