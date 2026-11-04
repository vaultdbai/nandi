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
 * "Connect to your company's relay": the one calm state of a
 * person-to-person place (asking or assigning someone, the directory,
 * sharing) while no relay is connected, or when an action answered
 * `RelayUnavailable` (`isRelayUnavailable`). Opens Network › Connection.
 * Never shown on local screens.
 *
 * `panel` fills an empty area; `inline` sits inside a form or dialog.
 */
export function RelayConnectNotice({
  variant = 'panel',
  className,
}: {
  variant?: 'panel' | 'inline'
  className?: string
}) {
  const { t } = useTranslation()

  if (variant === 'inline') {
    return (
      <div
        className={cn(
          'flex flex-wrap items-center gap-3 rounded-md border bg-muted/40 px-3 py-2',
          className
        )}
        data-testid="relay-connect-notice"
      >
        <Network
          aria-hidden
          className="size-4 shrink-0 text-muted-foreground"
        />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">{t('relayConnect.title')}</p>
          <p className="text-xs text-muted-foreground">
            {t('relayConnect.description')}
          </p>
        </div>
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={openConnection}
        >
          {t('relayConnect.action')}
        </Button>
      </div>
    )
  }

  return (
    <Empty className={className} data-testid="relay-connect-notice">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <Network aria-hidden />
        </EmptyMedia>
        <EmptyTitle>{t('relayConnect.title')}</EmptyTitle>
        <EmptyDescription>{t('relayConnect.description')}</EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button variant="outline" onClick={openConnection}>
          {t('relayConnect.action')}
        </Button>
      </EmptyContent>
    </Empty>
  )
}
