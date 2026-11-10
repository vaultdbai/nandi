import { useTranslation } from 'react-i18next'
import { HeartHandshake } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { formatRelativeTime } from '@/lib/format-time'
import { KNOW_ME_THREAD, useKnowMe } from '@/hooks/useKnowMe'
import { useNavigationStore } from '@/store/navigation-store'

/**
 * "Tell Nandi about you" (What you offer, My notes): opens the know-me chat
 * any time, whatever the to-do says, and shows what the latest answers
 * changed (the `knowMe` decisions).
 */
export function TellNandiAboutYou() {
  const { t, i18n } = useTranslation()
  const knowMe = useKnowMe()
  const recent = knowMe.data?.recent ?? []

  return (
    <section
      data-testid="tell-nandi-about-you"
      aria-labelledby="tell-nandi-title"
      className="grid gap-2 rounded-lg border p-4"
    >
      <div className="flex flex-wrap items-center gap-3">
        <HeartHandshake aria-hidden className="size-4 shrink-0 text-primary" />
        <div className="min-w-0 flex-1">
          <h2 id="tell-nandi-title" className="text-sm font-semibold">
            {t('knowMe.entry.title')}
          </h2>
          <p className="text-xs text-muted-foreground">
            {t('knowMe.entry.description')}
          </p>
        </div>
        <Button
          size="sm"
          variant="outline"
          onClick={() =>
            useNavigationStore.getState().navigate({
              kind: 'inbox-thread',
              conversationId: KNOW_ME_THREAD,
            })
          }
        >
          {t('knowMe.entry.open')}
        </Button>
      </div>
      {recent.length > 0 && (
        <div className="grid gap-1">
          <h3 className="text-xs font-medium text-muted-foreground">
            {t('knowMe.entry.recent')}
          </h3>
          <ul className="grid gap-1" data-testid="know-me-recent">
            {recent.map(record => (
              <li key={`${record.at}-${record.itemId}`} className="text-xs">
                <span className="text-muted-foreground">
                  {formatRelativeTime(record.at, i18n.language)}
                </span>{' '}
                <span dir="auto">{record.reason ?? record.itemLabel}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </section>
  )
}
