import { useTranslation } from 'react-i18next'
import { X } from 'lucide-react'
import { AskComposer } from '@/components/ask/AskComposer'
import { MarkdownLinkContext } from '@/components/agent-console/markdown-link-context'
import { MessageList } from '@/components/agent-console/MessageList'
import { RecentRow, RunningRow } from '@/components/home/HomeRows'
import { InboxItemCard } from '@/components/inbox/Inbox'
import type { InboxItem } from '@/components/inbox/inbox-model'
import type { HomeRow } from '@/components/home/home-model'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import type { UseCompanion } from '@/hooks/useCompanion'
import { useRequest } from '@/hooks/useRequests'
import { useViewOpener } from '@/hooks/useViewOpener'
import { decideApproval } from '@/lib/agent-actions'
import { displayAgent, STUDIO_AGENT_ID } from '@/lib/agent-identity'
import { cn } from '@/lib/utils'
import {
  CARD_LIMITS,
  requestIdFromLink,
  summaryParts,
  type SummaryCounts,
} from './companion-model'
import { CompanionHeader } from './CompanionHeader'
import { RequestCard } from './RequestCard'
import { handlePhrase } from '@/hooks/useMascot'

export interface CompanionFeed {
  /** Home's Inbox items, failures last */
  inbox: InboxItem[]
  running: HomeRow[]
  recent: HomeRow[]
  counts: SummaryCounts
  /** No agents and nothing has happened yet */
  firstRun: boolean
  loading: boolean
  now: number
}

export interface CompanionPanelProps {
  companion: UseCompanion
  feed: CompanionFeed
  /** Watch in companion: shown at the top of Running */
  focusedRequestId: string | null
  onClearFocus: () => void
  onOpenInNandi: () => void
  onOpenInStudio: () => void
  /** Expanded ↔ overlay */
  onToggleOverlay: () => void
  /** Back to the orb */
  onMinimize: () => void
  onHide: () => void
  /** Quits Nandi (standalone window only) */
  onQuit?: () => void
}

/**
 * The expanded companion card: a pocket Home (Needs you, Running, Recent,
 * from Home's own queries and row components) and a pocket Studio (one
 * input to the companion's grounded conversation)
 */
export function CompanionExpanded(props: CompanionPanelProps) {
  return <CompanionPanel {...props} large={false} />
}

/** Shared by the expanded card and the overlay */
export function CompanionPanel({
  companion,
  feed,
  focusedRequestId,
  onClearFocus,
  onOpenInNandi,
  onOpenInStudio,
  onToggleOverlay,
  onMinimize,
  onHide,
  onQuit,
  large,
}: CompanionPanelProps & { large: boolean }) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const noModel = companion.snapshot?.blocker === 'noModel'
  const waiting = feed.inbox.filter(item => item.kind !== 'failed')
  const needsYou = waiting.slice(0, CARD_LIMITS.needsYou)
  const running = feed.running.slice(0, CARD_LIMITS.running)
  const recent = feed.recent.slice(0, CARD_LIMITS.recent)
  const summary = summaryParts(feed.counts)
  const chatting =
    companion.messages.length > 0 || companion.scopeRequestId !== null

  // Requests cited in chat render as live cards
  const renderLink = (href: string) => {
    const id = requestIdFromLink(href)
    return id ? <RequestCard requestId={id} onAsk={companion.setScope} /> : null
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <CompanionHeader
        expanded={large}
        onToggleExpanded={onToggleOverlay}
        pinned={companion.pinned}
        onPinnedChange={pinned => void companion.setPinned(pinned)}
        notify={companion.notify}
        onNotifyChange={notify => void companion.setNotify(notify)}
        onOpenInNandi={onOpenInNandi}
        onOpenInStudio={onOpenInStudio}
        onHide={onHide}
        onMinimize={onMinimize}
        onQuit={onQuit}
      />
      <p
        className="shrink-0 border-b px-3 py-1.5 text-xs text-muted-foreground"
        data-testid="companion-summary"
      >
        {summary.length > 0
          ? summary
              .map(part =>
                t(`companion.summary.${part.key}`, { count: part.count })
              )
              .join(' · ')
          : t('companion.summary.quiet')}
      </p>
      <div
        className={cn(
          'flex min-h-0 flex-1 flex-col overflow-y-auto',
          large && 'md:flex-row'
        )}
      >
        <div
          className={cn(
            'flex flex-col gap-3 p-3',
            large && 'md:w-1/2 md:border-e'
          )}
        >
          {feed.loading ? (
            <div className="flex flex-col gap-2">
              <Skeleton className="h-10 w-full" />
              <Skeleton className="h-10 w-full" />
            </div>
          ) : feed.firstRun && waiting.length === 0 ? (
            <div className="flex flex-col items-start gap-2 text-sm">
              <p>{t('companion.firstRun')}</p>
              <Button
                size="sm"
                variant="outline"
                onClick={() => open({ kind: 'starter-packs' })}
              >
                {t('companion.installPack')}
              </Button>
            </div>
          ) : (
            <>
              {needsYou.length > 0 && (
                <Section
                  title={t('companion.needsYou', { count: waiting.length })}
                >
                  {needsYou.map(item => (
                    <li key={item.id}>
                      <InboxItemCard item={item} />
                    </li>
                  ))}
                  {waiting.length > needsYou.length && (
                    <MoreLink
                      count={waiting.length - needsYou.length}
                      onClick={() => open({ kind: 'home' })}
                    />
                  )}
                </Section>
              )}
              {(running.length > 0 || focusedRequestId) && (
                <Section
                  title={t('companion.running', { count: feed.running.length })}
                >
                  {focusedRequestId && (
                    <li className="flex items-start gap-1">
                      <span className="min-w-0 flex-1">
                        <RequestCard
                          requestId={focusedRequestId}
                          onAsk={companion.setScope}
                        />
                      </span>
                      <Button
                        size="icon-sm"
                        variant="ghost"
                        className="mt-2 size-6 shrink-0"
                        aria-label={t('companion.stopWatching')}
                        title={t('companion.stopWatching')}
                        onClick={onClearFocus}
                      >
                        <X />
                      </Button>
                    </li>
                  )}
                  {running.map(row => (
                    <li key={row.id}>
                      <RunningRow row={row} now={feed.now} />
                    </li>
                  ))}
                  {feed.running.length > running.length && (
                    <MoreLink
                      count={feed.running.length - running.length}
                      onClick={() => open({ kind: 'home' })}
                    />
                  )}
                </Section>
              )}
              {recent.length > 0 && (
                <Section title={t('companion.recent')}>
                  {recent.map(row => (
                    <li key={row.id}>
                      <RecentRow row={row} />
                    </li>
                  ))}
                </Section>
              )}
              {needsYou.length === 0 &&
                running.length === 0 &&
                recent.length === 0 &&
                !focusedRequestId && (
                  <p className="text-sm text-muted-foreground">
                    {t('companion.allQuiet')}
                  </p>
                )}
            </>
          )}
        </div>
        <div
          className={cn(
            'flex min-h-0 flex-col gap-2 border-t p-3',
            large && 'md:flex-1 md:border-t-0'
          )}
        >
          {companion.scopeRequestId && (
            <ScopeChip
              requestId={companion.scopeRequestId}
              onClear={() => companion.setScope(null)}
            />
          )}
          {chatting && companion.messages.length > 0 && (
            <div
              aria-label={t('companion.conversation')}
              className={cn('flex min-h-0 flex-col', large ? 'h-96' : 'h-56')}
            >
              <MarkdownLinkContext value={renderLink}>
                <MessageList
                  messages={companion.messages}
                  assistant={displayAgent('studio', t('companion.title'))}
                  working={companion.busy}
                  onDecide={(approvalId, decision) =>
                    decideApproval(STUDIO_AGENT_ID, approvalId, decision)
                  }
                  empty={null}
                />
              </MarkdownLinkContext>
            </div>
          )}
          {companion.sendError && (
            <p role="alert" className="text-xs text-destructive">
              {companion.sendError}
            </p>
          )}
          <AskComposer
            autoFocus
            placeholder={
              noModel
                ? t('companion.noModelPlaceholder')
                : t('companion.askPlaceholder')
            }
            disabled={noModel}
            busy={companion.busy}
            onSend={text => {
              // The hidden phrase is answered, not sent
              if (!handlePhrase(text)) void companion.send(text)
            }}
          />
        </div>
      </div>
    </div>
  )
}

/** "About: <Request>": the conversation is pinned to it */
function ScopeChip({
  requestId,
  onClear,
}: {
  requestId: string
  onClear: () => void
}) {
  const { t } = useTranslation()
  const { data: request } = useRequest(requestId)
  return (
    <div
      className="flex items-center gap-2 rounded-md border bg-muted/40 px-2 py-1 text-xs"
      data-testid="scope-chip"
    >
      <span className="min-w-0 flex-1 truncate">
        {t('companion.about', { title: request?.title ?? '…' })}
      </span>
      <Button
        size="icon-sm"
        variant="ghost"
        className="size-5"
        aria-label={t('companion.clearScope')}
        title={t('companion.clearScope')}
        onClick={onClear}
      >
        <X />
      </Button>
    </div>
  )
}

function Section({
  title,
  children,
}: {
  title: string
  children: React.ReactNode
}) {
  return (
    <section aria-label={title} className="flex min-w-0 flex-col gap-1.5">
      <h3 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        {title}
      </h3>
      <ul className="flex flex-col gap-1.5">{children}</ul>
    </section>
  )
}

function MoreLink({ count, onClick }: { count: number; onClick: () => void }) {
  const { t } = useTranslation()
  return (
    <li>
      <Button
        size="sm"
        variant="link"
        className="h-auto px-0 text-xs"
        onClick={onClick}
      >
        {t('companion.moreCount', { count })}
      </Button>
    </li>
  )
}
