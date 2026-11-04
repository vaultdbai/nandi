import { useTranslation } from 'react-i18next'
import { MarkdownContent } from '@/components/agent-console/MarkdownContent'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { sourceView } from '@/lib/ask'
import type { QuestionAnswer } from '@/lib/bindings'
import { useNavigationStore } from '@/store/navigation-store'

/** An inline answer: markdown, what it drew on, and follow-up questions */
export function AskAnswer({
  answer,
  onFollowup,
}: {
  answer: QuestionAnswer
  onFollowup?: (question: string) => void
}) {
  const { t } = useTranslation()

  return (
    <div className="flex flex-col gap-3">
      <div className="text-sm">
        <MarkdownContent content={answer.answer} />
      </div>
      {answer.sources.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-xs text-muted-foreground">
            {t('ask.sources')}
          </span>
          {answer.sources.map(source => {
            const view = sourceView(source)
            return (
              <Badge
                key={`${source.kind}:${source.id}`}
                variant="outline"
                asChild={view !== null}
              >
                {view ? (
                  <button
                    type="button"
                    onClick={() => useNavigationStore.getState().navigate(view)}
                  >
                    {source.label}
                  </button>
                ) : (
                  source.label
                )}
              </Badge>
            )
          })}
        </div>
      )}
      {answer.followups.length > 0 && onFollowup && (
        <ul aria-label={t('ask.followups')} className="flex flex-wrap gap-1.5">
          {answer.followups.map(question => (
            <li key={question}>
              <Button
                type="button"
                size="sm"
                variant="secondary"
                className="h-auto py-1 text-start text-xs whitespace-normal"
                onClick={() => onFollowup(question)}
              >
                {question}
              </Button>
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
