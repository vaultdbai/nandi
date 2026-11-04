import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { Pin, PinOff, Send, Sparkles } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardAction,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import {
  autoProvider,
  filterCapabilities,
  providerLabel,
  useCapabilityProviders,
  useSetProviderPin,
} from '@/hooks/useSharedAgents'
import type { CapabilityProviders, ProviderOption } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import type { CallRequest } from './CallDialog'

/**
 * "Who gets the work": every capability my agents and the agents shared
 * with me serve, with each provider's owner, reach, and track record, in
 * the order Auto would try them. Pinning one sends Auto tasks there first.
 */
export function CapabilityProvidersList({
  search,
  onCall,
}: {
  search: string
  onCall: (request: CallRequest) => void
}) {
  const { t } = useTranslation()
  const overview = useCapabilityProviders()

  if (overview.isPending) {
    return (
      <div className="flex flex-col gap-4">
        {[0, 1].map(i => (
          <Skeleton key={i} className="h-40" />
        ))}
      </div>
    )
  }
  if (overview.error) {
    return (
      <p className="text-sm text-destructive">
        {t('shared.loadError', { error: overview.error.message })}
      </p>
    )
  }
  const entries = filterCapabilities(overview.data, search)
  if (overview.data.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">
        {t('shared.capabilities.empty')}
      </p>
    )
  }
  if (entries.length === 0) {
    return (
      <p className="text-sm text-muted-foreground">{t('shared.noMatches')}</p>
    )
  }
  return (
    <div className="flex flex-col gap-4">
      {entries.map(entry => (
        <CapabilityCard key={entry.capability} entry={entry} onCall={onCall} />
      ))}
    </div>
  )
}

function CapabilityCard({
  entry,
  onCall,
}: {
  entry: CapabilityProviders
  onCall: (request: CallRequest) => void
}) {
  const { t } = useTranslation()
  const auto = autoProvider(entry)
  const pinnedMissing =
    entry.pinned !== null &&
    !entry.providers.some(
      provider => provider.card.id === entry.pinned && provider.rank !== null
    )

  return (
    <Card data-testid={`capability-${entry.capability}`} className="gap-3">
      <CardHeader>
        <CardTitle className="flex flex-wrap items-center gap-2 font-mono text-sm">
          {entry.capability}
          <Badge variant={entry.providers.length > 1 ? 'default' : 'secondary'}>
            {t('shared.capabilities.providers', {
              count: entry.providers.length,
            })}
          </Badge>
        </CardTitle>
        <CardDescription>
          {auto
            ? t('shared.capabilities.autoUses', { name: providerLabel(auto) })
            : t('shared.capabilities.noOneFree')}
          {pinnedMissing && (
            <span className="block text-warning">
              {t('shared.capabilities.pinUnavailable')}
            </span>
          )}
        </CardDescription>
        <CardAction>
          <Button
            size="sm"
            variant="outline"
            onClick={() =>
              onCall({
                title: t('shared.capabilities.sendTitle', {
                  capability: entry.capability,
                }),
                capabilities: [entry.capability],
                target: null,
              })
            }
          >
            <Send />
            {t('shared.capabilities.send')}
          </Button>
        </CardAction>
      </CardHeader>
      <CardContent>
        <ul className="flex flex-col divide-y rounded-md border">
          {entry.providers.map(provider => (
            <ProviderRow
              key={provider.card.id}
              capability={entry.capability}
              provider={provider}
              pinned={entry.pinned === provider.card.id}
              auto={entry.autoChoice === provider.card.id}
            />
          ))}
        </ul>
      </CardContent>
    </Card>
  )
}

function ProviderRow({
  capability,
  provider,
  pinned,
  auto,
}: {
  capability: string
  provider: ProviderOption
  pinned: boolean
  auto: boolean
}) {
  const { t } = useTranslation()
  const setPin = useSetProviderPin()
  const { card, stats } = provider
  const owner = card.owner.githubHandle
  const available = provider.rank !== null

  const togglePin = () =>
    setPin.mutate(
      { capability, cardId: pinned ? null : card.id },
      {
        onError: error =>
          toast.error(t('shared.capabilities.pinFailed'), {
            description: error.message,
          }),
      }
    )

  return (
    <li
      data-testid={`provider-${card.id}`}
      className={cn(
        'flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2',
        !available && 'text-muted-foreground'
      )}
    >
      <span className="w-6 text-xs text-muted-foreground tabular-nums">
        {provider.rank !== null ? `#${provider.rank}` : '–'}
      </span>
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="flex flex-wrap items-center gap-1.5 text-sm">
          <span className="truncate font-medium">{card.name}</span>
          <span className="text-muted-foreground">
            {owner ? `@${owner}` : t('shared.unknownOwner')}
          </span>
          {auto && (
            <Badge className="gap-1">
              <Sparkles className="size-3" />
              {t('shared.capabilities.autoBadge')}
            </Badge>
          )}
          {provider.mine && (
            <Badge variant="secondary">{t('shared.capabilities.mine')}</Badge>
          )}
          {provider.teammate && (
            <Badge variant="secondary">
              {t('shared.capabilities.teammate')}
            </Badge>
          )}
          {provider.sources.map(source => (
            <Badge key={source} variant="outline">
              {t(`shared.source.${source}`)}
            </Badge>
          ))}
          {provider.teamIds.map(teamId => (
            <Badge key={teamId} variant="outline" className="font-normal">
              {teamId}
            </Badge>
          ))}
        </span>
        {card.description && (
          <span className="line-clamp-1 text-xs text-muted-foreground">
            {card.description}
          </span>
        )}
        <span className="text-xs text-muted-foreground">
          {provider.unavailable
            ? t(`shared.capabilities.unavailable.${provider.unavailable}`)
            : t('shared.capabilities.load', {
                load: card.currentLoad,
                max: Math.max(card.maxConcurrent, 1),
              })}
          {' · '}
          {stats.calls === 0
            ? t('shared.capabilities.noHistory')
            : t('shared.capabilities.history', {
                count: stats.calls,
                rate:
                  provider.successRate === null
                    ? '–'
                    : `${Math.round(provider.successRate * 100)}%`,
              })}
          {stats.avgDurationMs !== null &&
            ` · ${t('shared.capabilities.avg', {
              seconds: (stats.avgDurationMs / 1000).toFixed(1),
            })}`}
        </span>
      </div>
      <Button
        size="sm"
        variant={pinned ? 'secondary' : 'ghost'}
        aria-pressed={pinned}
        disabled={setPin.isPending}
        onClick={togglePin}
        title={
          pinned
            ? t('shared.capabilities.unpinHint')
            : t('shared.capabilities.pinHint')
        }
      >
        {pinned ? <PinOff /> : <Pin />}
        {pinned ? t('shared.capabilities.unpin') : t('shared.capabilities.pin')}
      </Button>
    </li>
  )
}
