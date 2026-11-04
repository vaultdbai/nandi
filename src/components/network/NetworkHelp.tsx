import { useTranslation } from 'react-i18next'
import { CircleHelp } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import { useNetworkHealth } from '@/hooks/useNetworkHealth'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import type { NetworkHealth } from './reachability-model'

/** The words that get a one-line explanation (network.md §4.5) */
export type NetworkTerm =
  | 'relay'
  | 'listed'
  | 'trusted'
  | 'asksFirst'
  | 'byName'
  | 'needsApproval'
  | 'device'

/** A word with a dotted underline and a one-sentence tooltip */
export function Term({
  term,
  children,
}: {
  term: NetworkTerm
  children: React.ReactNode
}) {
  const { t } = useTranslation()
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          tabIndex={0}
          className="cursor-help underline decoration-dotted underline-offset-4"
        >
          {children}
        </span>
      </TooltipTrigger>
      <TooltipContent className="max-w-64">
        {t(`network.term.${term}`)}
      </TooltipContent>
    </Tooltip>
  )
}

/** "How networking works": five lines and a picture, on every tab */
export function HowNetworkingWorks() {
  const { t } = useTranslation()
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button size="sm" variant="ghost">
          <CircleHelp />
          {t('network.help.button')}
        </Button>
      </PopoverTrigger>
      <PopoverContent align="end" className="w-96 space-y-3 text-sm">
        <p className="font-medium">{t('network.help.title')}</p>
        <p
          dir="ltr"
          className="rounded-md bg-muted px-3 py-2 text-center text-xs text-muted-foreground"
          aria-label={t('network.help.diagramLabel')}
        >
          {t('network.help.diagram')}
        </p>
        <ol className="list-decimal space-y-1 ps-5">
          {[1, 2, 3, 4, 5].map(line => (
            <li key={line}>{t(`network.help.line${line}`)}</li>
          ))}
        </ol>
      </PopoverContent>
    </Popover>
  )
}

const HEALTH_DOT: Record<NetworkHealth, string> = {
  none: 'border border-muted-foreground/50',
  green: 'bg-success',
  amber: 'bg-warning',
  red: 'bg-destructive',
}

export function HealthDot({
  health,
  className,
}: {
  health: NetworkHealth
  className?: string
}) {
  return (
    <span
      aria-hidden
      className={cn(
        'size-2 shrink-0 rounded-full',
        HEALTH_DOT[health],
        className
      )}
    />
  )
}

/** The header pill: the nav dot in words; opens Connection */
export function NetworkStatusPill() {
  const { t } = useTranslation()
  const { health, label } = useNetworkHealth()
  return (
    <Button
      size="sm"
      variant="outline"
      className="gap-2"
      data-testid="network-status"
      onClick={() =>
        useNavigationStore
          .getState()
          .navigate({ kind: 'network', tab: 'connection' })
      }
    >
      <HealthDot health={health} />
      {t(label)}
    </Button>
  )
}
