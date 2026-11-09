import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { CircleCheck, Network, Sparkles } from 'lucide-react'
import {
  BackupNudge,
  GetStarted,
  WelcomeStrip,
} from '@/components/dashboard/FirstRunHints'
import { isFreshDashboard } from '@/components/dashboard/dashboard-model'
import { Inbox } from '@/components/inbox/Inbox'
import { LlmKeyNotice } from '@/components/layout/LlmKeyNotice'
import { RecentChanges } from '@/components/live/RecentChanges'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { useHomeFeed } from '@/hooks/useHome'
import { useRelay } from '@/hooks/useRelay'
import { sendToStudio } from '@/lib/agent-actions'
import { useDashboard } from '@/store/agent-store'
import { useNavigationStore, type View } from '@/store/navigation-store'
import { ViewHeader } from '@/components/views/ViewHeader'
import { ChiefOfStaffDigest } from './ChiefOfStaffDigest'
import { EmailTodo } from './EmailTodo'

const navigate = (view: View) => useNavigationStore.getState().navigate(view)

/**
 * Home: does anything need me? Ask Nandi, the Chief of Staff's digest and
 * email to-do, then the Inbox (fix it here). Running and finished work live
 * in the work tray on every page, and All activity in the sidebar. A fresh
 * install shows what to do first instead of empty sections.
 */
export function HomeView({
  welcome,
}: {
  /** Just set up: greet the packs installed */
  welcome?: { packIds: string[] }
}) {
  const { t } = useTranslation()
  const dashboard = useDashboard()
  const home = useHomeFeed()
  const inboxItems = home.inbox.items.length
  // "No model" is an Inbox item; on a fresh install it's the banner instead
  const waitingOnYou = home.inbox.items.filter(
    item => item.kind !== 'noModel'
  ).length
  const fresh =
    dashboard !== null &&
    isFreshDashboard(dashboard) &&
    waitingOnYou === 0 &&
    home.running.length === 0 &&
    home.recent.length === 0

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader title={t('nav.home')} />
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto flex w-full max-w-4xl flex-col gap-6 p-6">
          <AskNandi />
          {fresh && <LlmKeyNotice className="m-0" />}
          {welcome && welcome.packIds.length > 0 && (
            <WelcomeStrip packIds={welcome.packIds} />
          )}
          {dashboard && <BackupNudge snapshot={dashboard} />}
          <EmailTodo />
          <ChiefOfStaffDigest />
          {dashboard === null || home.isPending ? (
            <HomeSkeleton />
          ) : fresh ? (
            <GetStarted snapshot={dashboard} />
          ) : (
            <>
              <NetworkLine />
              {inboxItems > 0 ? (
                <Inbox />
              ) : (
                <p className="flex items-center gap-2 text-sm text-muted-foreground">
                  <CircleCheck aria-hidden className="size-4 text-success" />
                  {t('home.quiet')}
                </p>
              )}
              <RecentChanges />
            </>
          )}
        </div>
      </ScrollArea>
    </div>
  )
}

/** "Ask Nandi…": typed text opens Studio with it sent; empty just opens it */
function AskNandi() {
  const { t } = useTranslation()
  const [text, setText] = useState('')

  return (
    <form
      role="search"
      onSubmit={event => {
        event.preventDefault()
        const ask = text.trim()
        if (ask) {
          setText('')
          void sendToStudio(ask, [], null)
        } else {
          navigate({ kind: 'studio', conversationId: null })
        }
      }}
    >
      <div className="relative">
        <Sparkles
          aria-hidden
          className="pointer-events-none absolute start-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
        />
        <Input
          value={text}
          onChange={event => setText(event.target.value)}
          placeholder={t('home.askPlaceholder')}
          aria-label={t('home.askLabel')}
          className="h-10 ps-9"
        />
      </div>
    </form>
  )
}

/** One line about the network, never "Relay unavailable" */
function NetworkLine() {
  const { t } = useTranslation()
  const { status } = useRelay()
  if (status?.connected) {
    return (
      <p
        className="flex items-center gap-2 text-sm text-muted-foreground"
        data-testid="home-network"
      >
        <Network aria-hidden className="size-4 text-success" />
        {t('home.network.connected', {
          count: Math.max(0, status.instancesOnline - 1),
        })}
      </p>
    )
  }
  return (
    <p
      className="flex flex-wrap items-center gap-2 text-sm text-muted-foreground"
      data-testid="home-network"
    >
      <Network aria-hidden className="size-4" />
      {t('home.network.notConnected')}
      <Button
        size="sm"
        variant="link"
        className="h-auto p-0"
        onClick={() => navigate({ kind: 'network', tab: 'connection' })}
      >
        {t('home.network.join')}
      </Button>
    </p>
  )
}

function HomeSkeleton() {
  return (
    <div className="flex flex-col gap-3" data-testid="home-loading">
      <Skeleton className="h-20 w-full" />
      <Skeleton className="h-20 w-full" />
    </div>
  )
}
