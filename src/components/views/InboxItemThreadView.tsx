import { useTranslation } from 'react-i18next'
import { MessagesSquare } from 'lucide-react'
import { InboxItemCard } from '@/components/inbox/Inbox'
import { inboxRowLabel, inboxThreadId } from '@/components/inbox/inbox-model'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { errorMessage } from '@/hooks/useDelegation'
import { useInbox } from '@/hooks/useInbox'
import { useInboxThread } from '@/hooks/useInboxThread'
import { sendInboxMessage } from '@/lib/agent-actions'
import { ThreadComposer, ThreadConversation } from './ChiefOfStaffThread'
import { ViewHeader } from './ViewHeader'

/**
 * One Inbox item's Ask chat with the Chief of Staff (`services::inbox`).
 * Above: the item's card, its buttons the shortcuts. Below: the chat, where
 * the user asks about it, answers in their own words, or decides with
 * conditions; a decision the Chief of Staff proposes waits for the user's
 * confirm on its card in the chat.
 */
export function InboxItemThreadView({
  conversationId,
}: {
  conversationId: string
}) {
  const { t } = useTranslation()
  const thread = useInboxThread(conversationId)
  const { items } = useInbox()
  const item = items.find(i => inboxThreadId(i) === conversationId) ?? null

  if (thread.isPending) {
    return (
      <div className="flex flex-col gap-3 p-4">
        <Skeleton className="h-6 w-1/3" />
        <Skeleton className="h-24 w-full" />
      </div>
    )
  }
  if (thread.isError) {
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyTitle>{t('inboxThread.loadError.title')}</EmptyTitle>
          <EmptyDescription>{errorMessage(thread.error)}</EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  const label = item ? inboxRowLabel(item) : null
  // The know-me chat is one standing chat: always its own name
  const title =
    thread.data.kind === 'knowMe'
      ? t('knowMe.entry.title')
      : label
        ? 'text' in label
          ? label.text
          : t(label.key, label.values)
        : thread.data.title

  return (
    <section
      aria-label={t('inboxThread.label')}
      className="flex h-full min-h-0 flex-col"
    >
      <ViewHeader
        leading={
          <MessagesSquare
            aria-hidden
            className="size-4 text-muted-foreground"
          />
        }
        title={title}
        subtitle={t('inboxThread.subtitle')}
      />
      {(item || !thread.data.waiting) && (
        <div
          data-testid="inbox-thread-top"
          className="flex max-h-[45%] shrink-0 flex-col gap-3 overflow-y-auto border-b p-4"
        >
          {item && thread.data.waiting ? (
            <InboxItemCard item={item} />
          ) : (
            <p
              className="text-sm text-muted-foreground"
              data-testid="inbox-thread-decided"
            >
              {t('inboxThread.decided')}
            </p>
          )}
        </div>
      )}
      <ThreadConversation conversationId={conversationId} />
      <div className="shrink-0 border-t bg-background p-3">
        <ThreadComposer
          placeholder={t('inboxThread.placeholder')}
          onSend={text => void sendInboxMessage(conversationId, text)}
        />
      </div>
    </section>
  )
}
