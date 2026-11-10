import { useTranslation } from 'react-i18next'
import { Button } from '@/components/ui/button'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { useHelpTopics } from '@/hooks/useHelp'
import { helpTopicKeys } from '@/lib/help'
import { cn } from '@/lib/utils'
import { useUIStore } from '@/store/ui-store'
import { HelpTopicView } from './HelpTopicView'

/**
 * The built-in help guide in a window: every topic, one open. Opened from
 * "How?" next to the no-model and Docker notices (`useUIStore.openHelp`);
 * it needs no model and works offline.
 */
export function HelpGuideDialog() {
  const { t } = useTranslation()
  const open = useUIStore(state => state.helpOpen)
  const topicId = useUIStore(state => state.helpTopicId)
  const { data: topics } = useHelpTopics()
  const close = () => useUIStore.getState().closeHelp()
  const current = topics?.find(topic => topic.id === topicId) ?? topics?.[0]

  return (
    <Dialog open={open} onOpenChange={next => !next && close()}>
      <DialogContent className="max-h-[80vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{t('help.title')}</DialogTitle>
          <DialogDescription>{t('help.description')}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4 sm:flex-row">
          <nav
            aria-label={t('help.topicsLabel')}
            className="flex shrink-0 flex-row flex-wrap gap-1 sm:w-48 sm:flex-col"
          >
            {topics?.map(topic => (
              <Button
                key={topic.id}
                type="button"
                size="sm"
                variant="ghost"
                aria-current={topic.id === current?.id ? 'true' : undefined}
                className={cn(
                  'h-auto justify-start py-1 text-start text-xs whitespace-normal',
                  topic.id === current?.id && 'bg-accent'
                )}
                onClick={() => useUIStore.getState().openHelp(topic.id)}
              >
                {t(helpTopicKeys(topic.id).title)}
              </Button>
            ))}
          </nav>
          <div className="min-w-0 flex-1">
            {current && (
              <HelpTopicView
                key={current.id}
                topic={current}
                onNavigate={close}
              />
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  )
}
