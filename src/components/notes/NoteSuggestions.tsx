import { useTranslation } from 'react-i18next'
import { Check, Lightbulb, X } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { useAnswerSuggestion, useNoteSuggestions } from '@/hooks/useNotes'
import { sendToStudio } from '@/lib/agent-actions'

/**
 * My notes › Suggestions from Chief of Staff: Accept ticks one (and offers
 * to work on it in Studio), Dismiss removes it. Both are remembered, so
 * the Chief of Staff doesn't suggest it again.
 */
export function NoteSuggestions() {
  const { t } = useTranslation()
  const suggestions = useNoteSuggestions()
  const answer = useAnswerSuggestion()
  const open = suggestions.data ?? []
  if (open.length === 0) return null

  const decide = (id: string, accept: boolean) =>
    answer.mutate(
      { id, accept },
      {
        onSuccess: text => {
          if (!accept) return
          toast.success(t('noteSuggestions.accepted'), {
            action: {
              label: t('noteSuggestions.openStudio'),
              onClick: () => void sendToStudio(text, [], null),
            },
          })
        },
        onError: error =>
          toast.error(t('noteSuggestions.failed'), {
            description: error.message,
          }),
      }
    )

  return (
    <section
      aria-labelledby="note-suggestions"
      className="space-y-2 border-b px-6 py-3"
    >
      <h2
        id="note-suggestions"
        className="flex items-center gap-2 text-sm font-medium"
      >
        <Lightbulb aria-hidden className="size-4 text-muted-foreground" />
        {t('noteSuggestions.title')}
      </h2>
      <ul className="space-y-1">
        {open.map(suggestion => (
          <li
            key={suggestion.id}
            className="flex flex-wrap items-center gap-2 text-sm"
            data-testid="note-suggestion"
          >
            <span className="min-w-0 flex-1">{suggestion.text}</span>
            <Button
              size="sm"
              variant="outline"
              disabled={answer.isPending}
              onClick={() => decide(suggestion.id, true)}
            >
              <Check />
              {t('noteSuggestions.accept')}
            </Button>
            <Button
              size="sm"
              variant="ghost"
              disabled={answer.isPending}
              onClick={() => decide(suggestion.id, false)}
            >
              <X />
              {t('noteSuggestions.dismiss')}
            </Button>
          </li>
        ))}
      </ul>
    </section>
  )
}
