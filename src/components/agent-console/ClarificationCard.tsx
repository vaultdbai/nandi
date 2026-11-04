import { useTranslation } from 'react-i18next'
import { MessageCircleQuestion, UserRound } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import type { PendingClarification } from '@/lib/bindings'
import {
  DEFAULT_MAX_ROUNDS,
  roundOf,
  useEscalateClarification,
} from '@/hooks/useClarifications'
import { WorkingDots } from './WorkingDots'

export interface ClarificationCardProps {
  clarification: PendingClarification
  /** The answer, once it came back */
  answer?: string | null
  /** Defaults to the round in the question ID */
  round?: number
  /** The requester policy's `maxRounds` */
  maxRounds?: number
}

/**
 * One question a provider asked about a delegated task: the question (grey
 * pill), the answer once it arrives (cyan pill), the round counter, and
 * whether it waits for a person or the agent.
 */
export function ClarificationCard({
  clarification,
  answer = null,
  round,
  maxRounds = DEFAULT_MAX_ROUNDS,
}: ClarificationCardProps) {
  const { t } = useTranslation()
  const escalate = useEscalateClarification()
  const { question } = clarification
  const currentRound = round ?? roundOf(question.id) ?? 1
  const waitingForHuman = clarification.approvalId !== null
  const answered = answer !== null

  return (
    <Card className="gap-3 px-4 py-3 shadow-none">
      <div className="flex items-center gap-2">
        <MessageCircleQuestion
          aria-hidden
          className="size-4 shrink-0 text-muted-foreground"
        />
        <p className="flex-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">
          {t('clarification.label')}
        </p>
        <Badge variant="outline">
          {t('clarification.round', { round: currentRound, max: maxRounds })}
        </Badge>
      </div>

      <div className="flex flex-col items-start gap-2">
        <div
          data-testid="clarification-question"
          className="max-w-full rounded-2xl bg-muted px-3 py-1.5 text-start text-sm text-foreground"
        >
          {question.question}
        </div>
        {question.topic && (
          <span className="ps-3 text-xs text-muted-foreground">
            {t('clarification.topic', { topic: question.topic })}
          </span>
        )}
        {answered && (
          <div
            data-testid="clarification-answer"
            className="max-w-full self-end rounded-2xl bg-info/15 px-3 py-1.5 text-start text-sm text-info"
          >
            {answer}
          </div>
        )}
      </div>

      {!answered && (
        <div className="flex items-center gap-2 text-xs text-muted-foreground">
          {waitingForHuman ? (
            <>
              <UserRound aria-hidden className="size-3.5" />
              <span>{t('clarification.waitingForHuman')}</span>
            </>
          ) : (
            <>
              <span role="status" className="flex-1">
                <WorkingDots label={t('clarification.waitingForAgent')} />
              </span>
              <Button
                size="sm"
                variant="ghost"
                disabled={escalate.isPending}
                onClick={() =>
                  escalate.mutate({
                    taskId: clarification.taskId,
                    questionId: question.id,
                  })
                }
              >
                {t('clarification.askMe')}
              </Button>
            </>
          )}
        </div>
      )}
    </Card>
  )
}
