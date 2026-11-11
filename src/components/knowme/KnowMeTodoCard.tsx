import { useTranslation } from 'react-i18next'
import { HeartHandshake } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { rawError } from '@/lib/friendly-error'
import { useKnowMeTodo } from '@/hooks/useKnowMe'
import type { KnowMeTodoAction } from '@/lib/bindings'

/**
 * The know-me to-do on top of its chat: the question, and Later (back in a
 * week) or Skip (back in a month at the soonest). Answering is the chat
 * below; nothing here blocks or nags.
 */
export function KnowMeTodoCard({ recheck }: { recheck: boolean }) {
  const { t } = useTranslation()
  const todo = useKnowMeTodo()
  const act = (action: KnowMeTodoAction) =>
    todo.mutate(action, {
      onSuccess: () => toast.success(t(`knowMe.todo.${action}Done`)),
      onError: error => toast.error(rawError(error)),
    })

  return (
    <Card
      data-testid="know-me-todo"
      className="flex-row flex-wrap items-center gap-3 px-4 py-3 shadow-none"
    >
      <HeartHandshake aria-hidden className="size-4 shrink-0 text-primary" />
      <div className="min-w-0 flex-1">
        <p className="text-sm break-words">
          {recheck ? t('inbox.knowMe.recheck') : t('inbox.knowMe.first')}
        </p>
        <p className="text-xs text-muted-foreground">{t('knowMe.todo.hint')}</p>
      </div>
      <Button
        size="sm"
        variant="ghost"
        disabled={todo.isPending}
        onClick={() => act('skip')}
      >
        {t('knowMe.todo.skip')}
      </Button>
      <Button
        size="sm"
        variant="outline"
        disabled={todo.isPending}
        onClick={() => act('later')}
      >
        {t('knowMe.todo.later')}
      </Button>
    </Card>
  )
}
