import { useTranslation } from 'react-i18next'
import { Send } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { useTask } from '@/hooks/useTasks'
import type { PersonAsk } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { MarkdownContent } from './MarkdownContent'
import { answerText, askStage } from './person-ask-model'

const STAGE_CLASSES = {
  sent: 'text-muted-foreground',
  seen: 'border-info/30 bg-info/10 text-info',
  answered: 'border-success/30 bg-success/10 text-success',
  notAnswered: 'text-muted-foreground',
} as const

/** One ask to a person, as the relay tells us: sent, seen, answered */
function OneAsk({ ask }: { ask: PersonAsk }) {
  const { t } = useTranslation()
  const { data: task } = useTask(ask.taskId)
  const stage = task ? askStage(task.state) : 'sent'
  const answer = stage === 'answered' ? answerText(task?.result ?? null) : null

  return (
    <li className="flex flex-col gap-1">
      <div className="flex items-center gap-2 text-sm">
        <span className="font-medium">{ask.who}</span>
        <Badge variant="outline" className={cn(STAGE_CLASSES[stage])}>
          {t(`personAsk.${stage}`)}
        </Badge>
      </div>
      {answer && <MarkdownContent content={answer} />}
    </li>
  )
}

/** The asks a message went out as (`@Sam …`, `@Finance …`) */
export function PersonAskCard({ asks }: { asks: readonly PersonAsk[] }) {
  const { t } = useTranslation()
  const group = asks[0]?.group
  return (
    <section
      aria-label={t('personAsk.label')}
      className="ms-11 flex flex-col gap-2 rounded-lg border bg-card p-3"
    >
      <p className="flex items-center gap-2 text-xs text-muted-foreground">
        <Send aria-hidden className="size-3.5" />
        {group
          ? t('personAsk.toGroup', { group, count: asks.length })
          : t('personAsk.toPerson', { name: asks[0]?.who ?? '' })}
      </p>
      <ul className="flex flex-col gap-2">
        {asks.map(ask => (
          <OneAsk key={ask.taskId} ask={ask} />
        ))}
      </ul>
    </section>
  )
}
