import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { AlertTriangle, Send } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { useCapabilityProviderBreakdown } from '@/hooks/useCapabilityEvidence'
import type { CapabilityListing } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import {
  cardFaces,
  formatRate,
  successRateOf,
  visibilityOf,
} from './capability-model'
import { DelegateCapabilityDialog } from './DelegateCapabilityDialog'

/**
 * Dual-face card: provider face (online, load, success rate per local
 * provider) when this instance provides it, consumer face (providers
 * online, evidence, Delegate) when it uses or could use it; with neither,
 * a compact card with owner, description, and Details. Click opens the
 * detail view.
 */
export function CapabilityCard({ listing }: { listing: CapabilityListing }) {
  const { t } = useTranslation()
  const [delegating, setDelegating] = useState(false)
  const { definition } = listing
  const faces = cardFaces(listing)
  const compact = !faces.provider && !faces.consumer
  const name = definition.name || definition.id

  const open = () =>
    useNavigationStore
      .getState()
      .navigate({ kind: 'capability-detail', id: definition.id })

  return (
    <Card
      role="button"
      tabIndex={0}
      data-testid={`capability-${definition.id}`}
      aria-label={t('capabilities.card.open', { name })}
      className="cursor-pointer gap-3 py-4 transition-colors hover:bg-accent/40 focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
      onClick={open}
      onKeyDown={event => {
        if (event.target !== event.currentTarget) return
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          open()
        }
      }}
    >
      <CardHeader className="gap-1 px-4">
        <CardTitle className="flex min-w-0 items-center gap-2 text-sm">
          <span className="truncate">{name}</span>
          <span className="shrink-0 text-xs font-normal text-muted-foreground tabular-nums">
            v{definition.version}
          </span>
          <Badge variant="outline" className="ms-auto shrink-0">
            {t(`capabilities.visibility.${visibilityOf(listing)}`)}
          </Badge>
        </CardTitle>
        <CardDescription className="flex flex-wrap items-center gap-x-2 text-xs">
          <span>
            {t('capabilities.card.owner', {
              owner: definition.owner.displayName || definition.owner.id,
            })}
          </span>
          {listing.source !== 'local' && (
            <span title={listing.registryOrigin ?? undefined}>
              · {t(`capabilities.source.${listing.source}`)}
            </span>
          )}
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-3 px-4">
        {definition.description && (
          <p className="line-clamp-2 text-sm text-muted-foreground">
            {definition.description}
          </p>
        )}
        {listing.missingTools.length > 0 && (
          <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
            <AlertTriangle
              className="size-3.5 shrink-0 text-warning"
              aria-hidden
            />
            {t('capabilities.card.missingTools', {
              tools: listing.missingTools.join(', '),
            })}
          </p>
        )}
        {!compact && (
          <div
            className={cn(
              'grid gap-3',
              faces.provider && faces.consumer && 'sm:grid-cols-2'
            )}
          >
            {faces.provider && <ProviderFace capabilityId={definition.id} />}
            {faces.consumer && (
              <ConsumerFace
                listing={listing}
                onDelegate={() => setDelegating(true)}
              />
            )}
          </div>
        )}
        {compact && (
          <div>
            <Button
              size="sm"
              variant="outline"
              onClick={event => {
                event.stopPropagation()
                open()
              }}
            >
              {t('capabilities.card.details')}
            </Button>
          </div>
        )}
      </CardContent>
      {faces.consumer && (
        // The dialog is portalled, but React events still bubble to the card
        <div className="contents" onClick={event => event.stopPropagation()}>
          <DelegateCapabilityDialog
            capabilityId={definition.id}
            name={name}
            open={delegating}
            onOpenChange={setDelegating}
          />
        </div>
      )}
    </Card>
  )
}

function FaceTitle({ children }: { children: React.ReactNode }) {
  return (
    <h3 className="text-xs font-medium tracking-wide text-muted-foreground uppercase">
      {children}
    </h3>
  )
}

function Stat({ label, value }: { label: string; value: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-2 text-xs">
      <span className="truncate text-muted-foreground">{label}</span>
      <span className="shrink-0 font-medium tabular-nums">{value}</span>
    </div>
  )
}

/** This instance's providers: online, load, and each one's success rate */
function ProviderFace({ capabilityId }: { capabilityId: string }) {
  const { t } = useTranslation()
  const breakdown = useCapabilityProviderBreakdown(capabilityId)
  const local = (breakdown.data ?? []).filter(row => row.provider.local)
  const online = local.filter(row => row.online).length
  const load = local.reduce((sum, row) => sum + row.load, 0)
  const capacity = local.reduce(
    (sum, row) => sum + (row.provider.maxConcurrent ?? 0),
    0
  )

  return (
    <section
      className="flex flex-col gap-1.5 rounded-md border p-3"
      data-testid="provider-face"
    >
      <FaceTitle>{t('capabilities.card.providerFace')}</FaceTitle>
      <Stat
        label={t('capabilities.card.status')}
        value={
          <span className="inline-flex items-center gap-1.5">
            <span
              aria-hidden
              className={cn(
                'size-2 rounded-full',
                online > 0 ? 'bg-success' : 'bg-muted-foreground/40'
              )}
            />
            {online > 0
              ? t('capabilities.card.online', { count: online })
              : t('capabilities.card.offline')}
          </span>
        }
      />
      <Stat
        label={t('capabilities.card.load')}
        value={capacity > 0 ? `${load} / ${capacity}` : load}
      />
      {local.map(row => (
        <Stat
          key={`${row.provider.instanceId}/${row.provider.providerId}`}
          label={row.provider.displayName ?? row.provider.providerId}
          value={formatRate(successRateOf(row.evidence))}
        />
      ))}
    </section>
  )
}

/** Providers online, aggregate evidence, and Delegate */
function ConsumerFace({
  listing,
  onDelegate,
}: {
  listing: CapabilityListing
  onDelegate: () => void
}) {
  const { t } = useTranslation()
  return (
    <section
      className="flex flex-col gap-1.5 rounded-md border p-3"
      data-testid="consumer-face"
    >
      <FaceTitle>{t('capabilities.card.consumerFace')}</FaceTitle>
      <Stat
        label={t('capabilities.card.providersOnline')}
        value={listing.onlineProviders}
      />
      <Stat
        label={t('capabilities.card.successRate')}
        value={formatRate(listing.successRate)}
      />
      <Stat
        label={t('capabilities.card.invocations')}
        value={listing.invocations}
      />
      <Button
        size="sm"
        className="mt-1 self-start"
        disabled={listing.onlineProviders === 0}
        onClick={event => {
          event.stopPropagation()
          onDelegate()
        }}
      >
        <Send />
        {t('capabilities.card.delegate')}
      </Button>
    </section>
  )
}
