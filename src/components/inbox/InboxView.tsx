import { useTranslation } from 'react-i18next'
import { ScrollArea } from '@/components/ui/scroll-area'
import { ViewHeader } from '@/components/views/ViewHeader'
import { Inbox } from './Inbox'

/**
 * Home › Inbox on its own page: what the tray's "Needs you" opens, so the
 * Inbox has an address even when Home is busy with everything else.
 */
export function InboxView() {
  const { t } = useTranslation()
  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader title={t('inbox.title')} />
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto w-full max-w-4xl p-6">
          <Inbox />
        </div>
      </ScrollArea>
    </div>
  )
}
