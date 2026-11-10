import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { LifeBuoy } from 'lucide-react'
import { Button } from '@/components/ui/button'
import type { HelpAnswer, HelpTopic } from '@/lib/bindings'
import { helpTopicKeys } from '@/lib/help'
import { HelpTopicView } from './HelpTopicView'

/**
 * The built-in help guide's answer, where a model's answer would be (the
 * ask box, a chat sent with no model): why the guide answered, the topic
 * that matches, and other topics to open in place.
 */
export function HelpAnswerCard({
  answer,
  onNavigate,
}: {
  answer: HelpAnswer
  onNavigate?: () => void
}) {
  const { t } = useTranslation()
  const [shown, setShown] = useState<HelpTopic | null>(answer.topic)
  const others = [answer.topic, ...answer.related].filter(
    (topic): topic is HelpTopic => topic !== null && topic.id !== shown?.id
  )

  return (
    <div className="flex flex-col gap-3" data-testid="help-answer">
      <p className="flex items-start gap-2 text-xs text-muted-foreground">
        <LifeBuoy aria-hidden className="mt-0.5 size-3.5 shrink-0" />
        <span>
          {t(
            answer.reason === 'noModel'
              ? 'help.reason.noModel'
              : 'help.reason.modelFailed'
          )}
          {answer.detail && (
            <span className="mt-0.5 block" data-testid="help-detail">
              {answer.detail}
            </span>
          )}
        </span>
      </p>
      {shown ? (
        <HelpTopicView topic={shown} onNavigate={onNavigate} />
      ) : (
        <p className="text-sm">{t('help.noMatch')}</p>
      )}
      {others.length > 0 && (
        <div className="flex flex-col gap-1.5">
          <span className="text-xs text-muted-foreground">
            {t('help.more')}
          </span>
          <ul aria-label={t('help.more')} className="flex flex-wrap gap-1.5">
            {others.map(topic => (
              <li key={topic.id}>
                <Button
                  type="button"
                  size="sm"
                  variant="secondary"
                  className="h-auto py-1 text-start text-xs whitespace-normal"
                  onClick={() => setShown(topic)}
                >
                  {t(helpTopicKeys(topic.id).title)}
                </Button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}
