import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { CheckCircle2, ChevronDown, CircleAlert } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { Spinner } from '@/components/ui/spinner'
import type { PackManifest } from '@/lib/bindings'
import type { InstallStatus } from '@/lib/marketplace-actions'
import { formatCostRange, formatSize } from '@/lib/marketplace-format'
import { cn } from '@/lib/utils'
import { installKey, useMarketplaceStore } from '@/store/marketplace-store'
import { CapabilityBadges } from './CapabilityBadges'
import { MarketplaceIcon } from './MarketplaceIcon'
import { VerificationBadge } from './VerificationBadge'

interface PackCardProps {
  pack: PackManifest
  /** Multi-select (starter packs) */
  selection?: {
    checked: boolean
    onCheckedChange: (checked: boolean) => void
    disabled?: boolean
  }
  /** While installing */
  status?: InstallStatus
  /** e.g. an Install button (Discover) */
  action?: React.ReactNode
}

/** A pack: what it contains, what it needs, and roughly what it costs. */
export function PackCard({ pack, selection, status, action }: PackCardProps) {
  const { t, i18n } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const size = formatSize(pack.installSizeKb, i18n.language)
  const cost = formatCostRange(pack.estimatedCostUsd, i18n.language)
  const installed = pack.installedVersion !== null

  return (
    <Card
      className={cn(
        'gap-3 shadow-none transition-colors',
        selection?.checked && 'border-primary ring-1 ring-primary'
      )}
    >
      <CardHeader>
        <div className="flex min-w-0 items-center gap-3">
          <MarketplaceIcon url={pack.iconUrl} kind="pack" />
          <div className="min-w-0">
            <CardTitle className="truncate">{pack.name}</CardTitle>
            <p className="mt-1 truncate text-xs text-muted-foreground">
              {t('marketplace.byPublisher', { publisher: pack.publisher })}
            </p>
          </div>
        </div>
        <CardAction>
          {selection ? (
            <Checkbox
              checked={selection.checked}
              disabled={selection.disabled}
              onCheckedChange={checked =>
                selection.onCheckedChange(checked === true)
              }
              aria-label={t('starterPacks.select', { name: pack.name })}
            />
          ) : installed ? (
            <Badge variant="secondary">
              {t('marketplace.installedVersion', {
                version: pack.installedVersion,
              })}
            </Badge>
          ) : null}
        </CardAction>
      </CardHeader>

      <CardContent className="flex flex-1 flex-col gap-3">
        <CardDescription className="line-clamp-3">
          {pack.description}
        </CardDescription>
        <div>
          <VerificationBadge verified={pack.verified} />
        </div>

        <p className="text-xs text-muted-foreground">
          {[
            t('marketplace.agentCount', { count: pack.agents.length }),
            size,
            cost && t('marketplace.costPerRun', { cost }),
          ]
            .filter(Boolean)
            .join(' · ')}
        </p>

        <Collapsible open={expanded} onOpenChange={setExpanded}>
          <CollapsibleTrigger className="flex items-center gap-1 text-xs font-medium text-foreground hover:underline">
            <ChevronDown
              className={cn(
                'size-3.5 transition-transform',
                expanded && 'rotate-180'
              )}
            />
            {expanded
              ? t('marketplace.hideContents')
              : t('marketplace.showContents')}
          </CollapsibleTrigger>
          <CollapsibleContent>
            <ul className="mt-2 flex flex-col gap-1.5 text-xs">
              {pack.agents.map(agent => (
                <li key={agent.id}>
                  <span className="font-medium text-foreground">
                    {agent.name}
                  </span>
                  {agent.description && (
                    <span className="text-muted-foreground">
                      {' '}
                      — {agent.description}
                    </span>
                  )}
                </li>
              ))}
              {pack.workflows.map(workflow => (
                <li key={workflow} className="text-muted-foreground">
                  {t('marketplace.workflowItem', { id: workflow })}
                </li>
              ))}
            </ul>
          </CollapsibleContent>
        </Collapsible>

        <CapabilityBadges capabilities={pack.capabilities} />

        {!pack.compatible && (
          <p className="flex items-center gap-1.5 text-xs text-destructive">
            <CircleAlert className="size-3.5" />
            {t('marketplace.needsNewerNandi', {
              version: pack.minNandiVersion,
            })}
          </p>
        )}
      </CardContent>

      {(status ?? action) && (
        <CardFooter className="flex-col items-stretch gap-2">
          {status && <PackStatus packId={pack.id} status={status} />}
          {action}
        </CardFooter>
      )}
    </Card>
  )
}

function PackStatus({
  packId,
  status,
}: {
  packId: string
  status: InstallStatus
}) {
  const { t } = useTranslation()
  const progress = useMarketplaceStore(
    state => state.progress[installKey('pack', packId)]
  )

  switch (status.state) {
    case 'queued':
      return (
        <p className="text-xs text-muted-foreground">
          {t('starterPacks.queued')}
        </p>
      )
    case 'installing': {
      const percent =
        progress && progress.total > 0
          ? Math.round((progress.received / progress.total) * 100)
          : null
      return (
        <div className="flex flex-col gap-1.5">
          <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <Spinner className="size-3.5" />
            {percent === null
              ? t('starterPacks.installing')
              : t('starterPacks.downloading', { percent })}
          </p>
          <div
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent ?? undefined}
            className="h-1 overflow-hidden rounded-full bg-muted"
          >
            <div
              className={cn(
                'h-full bg-primary transition-[width]',
                percent === null && 'w-1/3 animate-pulse'
              )}
              style={percent === null ? undefined : { width: `${percent}%` }}
            />
          </div>
        </div>
      )
    }
    case 'installed':
      return (
        <p className="flex items-center gap-1.5 text-xs text-foreground">
          <CheckCircle2 className="size-3.5 text-success" />
          {t('starterPacks.installed')}
        </p>
      )
    case 'failed':
      return (
        <p
          role="alert"
          className="flex items-start gap-1.5 text-xs text-destructive"
        >
          <CircleAlert className="mt-0.5 size-3.5 shrink-0" />
          {status.error}
        </p>
      )
  }
}
