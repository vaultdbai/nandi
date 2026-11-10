import { useTranslation } from 'react-i18next'
import { ExternalLink } from 'lucide-react'
import { MarkdownContent } from '@/components/agent-console/MarkdownContent'
import { Button } from '@/components/ui/button'
import type { HelpTopic } from '@/lib/bindings'
import { client } from '@/lib/client'
import { helpDestination, helpTopicKeys } from '@/lib/help'
import { useNavigationStore } from '@/store/navigation-store'

/**
 * One topic of the built-in help guide: its title, a short answer, and
 * buttons to the right screen (or page). `onNavigate` runs after a link
 * goes somewhere in the app (a dialog closes itself).
 */
export function HelpTopicView({
  topic,
  onNavigate,
}: {
  topic: HelpTopic
  onNavigate?: () => void
}) {
  const { t } = useTranslation()
  const keys = helpTopicKeys(topic.id)

  const follow = (target: string) => {
    const destination = helpDestination(target)
    if (!destination) return
    if (destination.kind === 'url') {
      void client.openExternal(destination.url)
      return
    }
    useNavigationStore.getState().navigate(destination.view)
    onNavigate?.()
  }

  return (
    <div className="flex flex-col gap-2" data-testid={`help-topic-${topic.id}`}>
      <h3 className="text-sm font-semibold">{t(keys.title)}</h3>
      <div className="text-sm">
        <MarkdownContent content={t(keys.answer)} />
      </div>
      {topic.links.length > 0 && (
        <div className="flex flex-wrap gap-1.5">
          {topic.links.map(link => (
            <Button
              key={link.target}
              type="button"
              size="sm"
              variant="outline"
              className="h-7"
              onClick={() => follow(link.target)}
            >
              {t(`help.links.${link.label}`)}
              {link.target.startsWith('url:') && (
                <ExternalLink aria-hidden className="size-3" />
              )}
            </Button>
          ))}
        </div>
      )}
    </div>
  )
}
