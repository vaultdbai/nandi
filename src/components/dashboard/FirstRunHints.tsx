import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  ArrowRight,
  CheckCircle2,
  DatabaseBackup,
  Network,
  Package,
  PencilRuler,
  X,
} from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { useAgents } from '@/hooks/useAgents'
import { useMarketplacePacks } from '@/hooks/useMarketplace'
import { useRelay } from '@/hooks/useRelay'
import { useRemoteStatus } from '@/hooks/useRemote'
import { sendToStudio } from '@/lib/agent-actions'
import { dismiss, isDismissed } from '@/lib/dismissals'
import { openSettings } from '@/lib/settings-nav'
import type { DashboardSnapshot } from '@/lib/tauri-bindings'
import { useNavigationStore, type View } from '@/store/navigation-store'
import {
  BACKUP_NUDGE_KEY,
  BACKUP_SNOOZE_MS,
  GET_STARTED_KEY,
  getStartedCards,
  showBackupNudge,
  welcomeFor,
  type GetStartedCard,
} from './dashboard-model'

const navigate = (view: View) => useNavigationStore.getState().navigate(view)

const CARD_ICONS: Record<GetStartedCard, React.ReactNode> = {
  packs: <Package />,
  studio: <PencilRuler />,
  network: <Network />,
}

const CARD_TARGETS: Record<GetStartedCard, View> = {
  packs: { kind: 'starter-packs' },
  studio: { kind: 'studio', conversationId: null },
  network: { kind: 'network', tab: 'connection' },
}

/** First run: one card per thing still worth doing, each with its button */
export function GetStarted({ snapshot }: { snapshot: DashboardSnapshot }) {
  const { t } = useTranslation()
  const relay = useRelay()
  const [dismissed, setDismissed] = useState(() => isDismissed(GET_STARTED_KEY))
  const cards = getStartedCards({
    snapshot,
    relayConnected: relay.status?.connected ?? false,
    dismissed,
  })
  if (cards.length === 0) return null

  return (
    <section
      aria-labelledby="get-started"
      className="flex flex-col gap-3 sm:col-span-2 xl:col-span-4"
    >
      <div className="flex items-center justify-between gap-2">
        <h2 id="get-started" className="text-base font-semibold">
          {t('dashboard.getStarted.title')}
        </h2>
        <Button
          variant="ghost"
          size="icon"
          className="size-7"
          aria-label={t('dashboard.getStarted.dismiss')}
          onClick={() => {
            dismiss(GET_STARTED_KEY)
            setDismissed(true)
          }}
        >
          <X />
        </Button>
      </div>
      <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
        {cards.map(card => (
          <Card
            key={card}
            className="gap-3"
            data-testid={`get-started-${card}`}
          >
            <CardHeader>
              <CardTitle className="flex items-center gap-2 [&_svg]:size-4 [&_svg]:text-muted-foreground">
                {CARD_ICONS[card]}
                {t(`dashboard.getStarted.${card}.title`)}
              </CardTitle>
              <CardDescription>
                {t(`dashboard.getStarted.${card}.description`)}
              </CardDescription>
            </CardHeader>
            <CardFooter className="mt-auto">
              <Button
                size="sm"
                variant={card === 'packs' ? 'default' : 'outline'}
                onClick={() => navigate(CARD_TARGETS[card])}
              >
                {t(`dashboard.getStarted.${card}.action`)}
              </Button>
            </CardFooter>
          </Card>
        ))}
      </div>
    </section>
  )
}

/**
 * Right after setup: what was installed and something to try. Shown once
 * (it rides on the navigation that ends onboarding).
 */
export function WelcomeStrip({ packIds }: { packIds: string[] }) {
  const { t } = useTranslation()
  const packs = useMarketplacePacks()
  const { data: agents = [] } = useAgents()
  const [hidden, setHidden] = useState(false)
  if (hidden) return null

  const welcome = welcomeFor(
    packIds,
    packs.data,
    agents.filter(agent => agent.builtin).map(agent => agent.name)
  )
  const suggestions =
    welcome.suggestions.length > 0
      ? welcome.suggestions
      : [t('dashboard.welcome.defaultAsk')]

  return (
    <section
      role="status"
      className="flex flex-col gap-3 rounded-lg border border-success/30 bg-success/5 p-4 sm:col-span-2 xl:col-span-4"
    >
      <div className="flex items-start justify-between gap-2">
        <p className="flex items-center gap-2 text-sm font-medium">
          <CheckCircle2 aria-hidden className="size-4 shrink-0 text-success" />
          {welcome.packNames.length > 0
            ? t('dashboard.welcome.installed', {
                count: welcome.agentNames.length,
                packs: welcome.packNames.join(', '),
              })
            : t('dashboard.welcome.setUp')}
        </p>
        <Button
          variant="ghost"
          size="icon"
          className="size-7"
          aria-label={t('dashboard.welcome.dismiss')}
          onClick={() => setHidden(true)}
        >
          <X />
        </Button>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm text-muted-foreground">
          {t('dashboard.welcome.try')}
        </span>
        {suggestions.map(text => (
          <Button
            key={text}
            size="sm"
            variant="outline"
            onClick={() => void sendToStudio(text, [], null)}
          >
            {text}
          </Button>
        ))}
      </div>
      {welcome.allAgentNames.length > 0 && (
        <p className="text-sm text-muted-foreground">
          {t('dashboard.welcome.agents', {
            names: welcome.allAgentNames.join(' · '),
          })}
        </p>
      )}
    </section>
  )
}

/** Once work has finished and nothing is backed up: one dismissible line */
export function BackupNudge({ snapshot }: { snapshot: DashboardSnapshot }) {
  const { t } = useTranslation()
  const remote = useRemoteStatus()
  const [dismissed, setDismissed] = useState(() =>
    isDismissed(BACKUP_NUDGE_KEY)
  )
  if (
    !showBackupNudge({
      snapshot,
      remoteConfigured: remote.data?.configured,
      dismissed,
    })
  ) {
    return null
  }

  return (
    <section className="flex flex-wrap items-center gap-3 rounded-lg border px-4 py-3 text-sm sm:col-span-2 xl:col-span-4">
      <DatabaseBackup
        aria-hidden
        className="size-4 shrink-0 text-muted-foreground"
      />
      <p className="min-w-0 flex-1">{t('dashboard.backupNudge.message')}</p>
      <Button size="sm" onClick={() => openSettings('backup')}>
        {t('dashboard.backupNudge.setUp')}
        <ArrowRight className="rtl:rotate-180" />
      </Button>
      <Button
        size="sm"
        variant="ghost"
        onClick={() => {
          dismiss(BACKUP_NUDGE_KEY, BACKUP_SNOOZE_MS)
          setDismissed(true)
        }}
      >
        {t('dashboard.backupNudge.notNow')}
      </Button>
    </section>
  )
}
