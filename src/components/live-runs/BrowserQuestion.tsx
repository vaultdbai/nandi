import { useTranslation } from 'react-i18next'
import { Globe } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useNow } from '@/components/agents/task-utils'
import { answerBrowserQuestion } from '@/lib/agent-actions'
import { useStopStore } from '@/store/stop-store'

/**
 * After a stop, while the agent's browser is still open: "Close the browser
 * too?" With no answer it closes on its own when the time is up (the core
 * does that); Keep it open leaves the window and its sites.
 */
export function BrowserQuestion() {
  const { t } = useTranslation()
  const question = useStopStore(state => state.question)
  const now = useNow(question !== null, 1_000)
  if (!question || now >= question.until) return null
  const seconds = Math.ceil((question.until - now) / 1_000)

  return (
    <div
      role="alertdialog"
      aria-label={t('stop.browserQuestion')}
      className="flex flex-wrap items-center gap-2 border-b bg-warning/5 px-4 py-2 text-sm"
    >
      <Globe aria-hidden className="size-4 shrink-0 text-warning" />
      <span className="min-w-0 flex-1">
        {t('stop.browserQuestion')}{' '}
        <span className="text-xs text-muted-foreground">
          {t('stop.browserClosesIn', { seconds })}
        </span>
      </span>
      <Button
        size="sm"
        variant="outline"
        onClick={() => void answerBrowserQuestion(question.id, false)}
      >
        {t('stop.keepOpen')}
      </Button>
      <Button
        size="sm"
        onClick={() => void answerBrowserQuestion(question.id, true)}
      >
        {t('stop.close')}
      </Button>
    </div>
  )
}
