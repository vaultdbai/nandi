import { useTranslation } from 'react-i18next'
import { Network } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'

const openConnection = () =>
  useNavigationStore.getState().navigate({ kind: 'network', tab: 'connection' })

/**
 * Why a person-to-person place can't be used: no relay connected
 * (`connect`), or the connected relay is too old for it (`update`)
 */
export type RelayNoticeReason = 'connect' | 'update'

/** The words of each reason, and whether it offers Network › Connection */
const WORDS: Record<
  RelayNoticeReason,
  { title: string; description: string; opensConnection: boolean }
> = {
  connect: {
    title: 'relayConnect.title',
    description: 'relayConnect.description',
    opensConnection: true,
  },
  // Only the relay's operator can update it: nothing to open here
  update: {
    title: 'relayUpdate.title',
    description: 'relayUpdate.description',
    opensConnection: false,
  },
}

/**
 * The one calm state of a person-to-person place (asking or assigning
 * someone, the directory, sharing, groups):
 *
 * - `connect`: "Connect to your company's relay", while no relay is
 *   connected, or when an action answered `RelayUnavailable`
 *   (`isRelayUnavailable`). Opens Network › Connection.
 * - `update`: "Your relay needs an update for this", when the connected
 *   relay is one version older and lacks the feature (`missingFeatures`,
 *   `isRelayNeedsUpdate`).
 *
 * Never shown on local screens. `panel` fills an empty area; `inline` sits
 * inside a form or dialog.
 */
export function RelayConnectNotice({
  variant = 'panel',
  reason = 'connect',
  className,
}: {
  variant?: 'panel' | 'inline'
  reason?: RelayNoticeReason
  className?: string
}) {
  const { t } = useTranslation()
  const words = WORDS[reason]
  const testId = `relay-${reason}-notice`

  if (variant === 'inline') {
    return (
      <div
        className={cn(
          'flex flex-wrap items-center gap-3 rounded-md border bg-muted/40 px-3 py-2',
          className
        )}
        data-testid={testId}
      >
        <Network
          aria-hidden
          className="size-4 shrink-0 text-muted-foreground"
        />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">{t(words.title)}</p>
          <p className="text-xs text-muted-foreground">
            {t(words.description)}
          </p>
        </div>
        {words.opensConnection && (
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={openConnection}
          >
            {t('relayConnect.action')}
          </Button>
        )}
      </div>
    )
  }

  return (
    <Empty className={className} data-testid={testId}>
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Network aria-hidden />
        </EmptyMedia>
        <EmptyTitle>{t(words.title)}</EmptyTitle>
        <EmptyDescription>{t(words.description)}</EmptyDescription>
      </EmptyHeader>
      {words.opensConnection && (
        <EmptyContent>
          <Button variant="outline" onClick={openConnection}>
            {t('relayConnect.action')}
          </Button>
        </EmptyContent>
      )}
    </Empty>
  )
}
