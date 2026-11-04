import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ArrowRight, CircleCheck, Network, Sparkles } from 'lucide-react'
import {
  BackupNudge,
  GetStarted,
  WelcomeStrip,
} from '@/components/dashboard/FirstRunHints'
import { isFreshDashboard } from '@/components/dashboard/dashboard-model'
import { Inbox } from '@/components/inbox/Inbox'
import { LlmKeyNotice } from '@/components/layout/LlmKeyNotice'
import { Button } from '@/components/ui/button'
import { Card } from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { useHomeFeed } from '@/hooks/useHome'
import { useRelay } from '@/hooks/useRelay'
import { sendToStudio } from '@/lib/agent-actions'
import { cn } from '@/lib/utils'
import { useDashboard } from '@/store/agent-store'
import { useNavigationStore, type View } from '@/store/navigation-store'
import { ViewHeader } from '@/components/views/ViewHeader'
import { RecentRow, RunningRow } from './HomeRows'
import type { HomeTile } from './home-model'

const navigate = (view: View) => useNavigationStore.getState().navigate(view)

/**
 * Home: does anything need me, and is everything else moving? Ask Nandi,
 * then Inbox (fix it here), Running (step n of m), Recent (result, rate,
 * retry), with All activity one click down. A fresh install shows what to
 * do first instead of empty sections; quiet sections collapse to one line.
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
  const quiet = inboxItems === 0 && home.running.length === 0

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
          {dashboard === null || home.isPending ? (
            <HomeSkeleton />
          ) : fresh ? (
            <GetStarted snapshot={dashboard} />
          ) : (
            <>
              <NetworkLine />
              {home.tiles.length > 0 && <Tiles tiles={home.tiles} />}
              {inboxItems > 0 && (
                <div id="home-inbox">
                  <Inbox />
                </div>
              )}
              {quiet ? (
                <p className="flex items-center gap-2 text-sm text-muted-foreground">
                  <CircleCheck aria-hidden className="size-4 text-success" />
                  {t('home.quiet')}
                </p>
              ) : (
                home.running.length > 0 && (
                  <Section
                    title={t('home.running.title', {
                      count: home.running.length,
                    })}
                  >
                    {home.running.map(row => (
                      <li key={row.id}>
                        <RunningRow row={row} now={home.now} />
                      </li>
                    ))}
                  </Section>
                )
              )}
              <Section
                title={t('home.recent.title')}
                action={
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() => navigate({ kind: 'home-activity' })}
                  >
                    {t('home.allActivity')}
                    <ArrowRight className="rtl:rotate-180" />
                  </Button>
                }
              >
                {home.recent.length === 0 ? (
                  <li className="flex flex-wrap items-center gap-3 px-3 py-2 text-sm text-muted-foreground">
                    {t('home.recent.empty')}
                    <Button
                      size="sm"
                      variant="outline"
                      className="h-7"
                      onClick={() =>
                        navigate({ kind: 'studio', conversationId: null })
                      }
                    >
                      {t('home.askLabel')}
                    </Button>
                  </li>
                ) : (
                  home.recent.map(row => (
                    <li key={row.id}>
                      <RecentRow row={row} />
                    </li>
                  ))
                )}
              </Section>
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

/** At most three, each opening All activity filtered (Waiting: the Inbox) */
function Tiles({ tiles }: { tiles: HomeTile[] }) {
  const { t } = useTranslation()
  return (
    <ul className="grid grid-cols-1 gap-3 sm:grid-cols-3">
      {tiles.map(tile => (
        <li key={tile.id}>
          <Card className="p-0">
            <button
              type="button"
              data-testid={`home-tile-${tile.id}`}
              onClick={() =>
                tile.filters
                  ? navigate({ kind: 'home-activity', filters: tile.filters })
                  : document
                      .getElementById('home-inbox')
                      ?.scrollIntoView({ behavior: 'smooth' })
              }
              className="flex w-full flex-col items-start gap-0.5 rounded-xl px-4 py-3 text-start hover:bg-accent/40"
            >
              <span className="text-xs font-medium text-muted-foreground">
                {t(`home.tile.${tile.id}`)}
              </span>
              <span
                className={cn(
                  'text-2xl font-semibold tabular-nums',
                  tile.id === 'failed' && tile.value > 0 && 'text-destructive',
                  tile.id === 'waiting' && tile.value > 0 && 'text-warning'
                )}
              >
                {tile.value}
              </span>
            </button>
          </Card>
        </li>
      ))}
    </ul>
  )
}

function Section({
  title,
  action,
  children,
}: {
  title: string
  action?: React.ReactNode
  children: React.ReactNode
}) {
  return (
    <section aria-label={title} className="flex flex-col gap-1">
      <div className="flex items-center justify-between gap-2">
        <h2 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
          {title}
        </h2>
        {action}
      </div>
      <ul className="flex flex-col divide-y rounded-lg border">{children}</ul>
    </section>
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
