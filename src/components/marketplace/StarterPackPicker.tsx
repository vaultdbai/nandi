import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { PackageOpen } from 'lucide-react'
import { PackCard } from '@/components/marketplace/PackCard'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Skeleton } from '@/components/ui/skeleton'
import { Spinner } from '@/components/ui/spinner'
import { useMarketplacePacks } from '@/hooks/useMarketplace'
import { installPacks, type InstallStatus } from '@/lib/marketplace-actions'
import { cn } from '@/lib/utils'

/** Starting with more than this is overwhelming; Library has the rest */
export const MAX_STARTER_PACKS = 3
/** First-party packs carry this tag and start selected (the user can untick) */
export const OFFICIAL_TAG = 'official'

type Phase = 'choosing' | 'installing' | 'done'

export interface StarterPackPickerProps {
  /** Every chosen pack was handled; `packIds` lists the ones that installed */
  onDone: (packIds: string[]) => void
  /** Leave without installing anything */
  onSkip: () => void
  /** Onboarding's Back (disabled while installing) */
  onBack?: () => void
  /** Onboarding's card is narrow: two columns at most */
  compact?: boolean
}

/**
 * Pick 1–3 curated packs and install them in one go, with per-pack status.
 * A pack that fails can be retried or left out; it never blocks the rest.
 */
export function StarterPackPicker({
  onDone,
  onSkip,
  onBack,
  compact,
}: StarterPackPickerProps) {
  const { t } = useTranslation()
  const packs = useMarketplacePacks()
  // `null` until the user changes the choice: official packs start ticked
  const [chosen, setSelected] = useState<string[] | null>(null)
  const [statuses, setStatuses] = useState<Record<string, InstallStatus>>({})
  const [phase, setPhase] = useState<Phase>('choosing')
  const [limitHit, setLimitHit] = useState(false)

  // Already-installed packs can't be picked again
  const available = (packs.data ?? []).filter(
    pack => pack.installedVersion === null || pack.id in statuses
  )
  const selected =
    chosen ??
    available
      .filter(pack => pack.tags.includes(OFFICIAL_TAG))
      .map(pack => pack.id)
      .slice(0, MAX_STARTER_PACKS)

  const toggle = (packId: string, checked: boolean) => {
    if (checked && selected.length >= MAX_STARTER_PACKS) {
      setLimitHit(true)
      return
    }
    setLimitHit(false)
    setSelected(
      checked
        ? [...selected, packId]
        : selected.filter(selectedId => selectedId !== packId)
    )
  }

  const installed = (all: Record<string, InstallStatus>) =>
    Object.entries(all)
      .filter(([, status]) => status.state === 'installed')
      .map(([packId]) => packId)

  const install = async (packIds: string[]) => {
    setPhase('installing')
    const next = { ...statuses }
    const allInstalled = await installPacks(packIds, (packId, status) => {
      next[packId] = status
      setStatuses(current => ({ ...current, [packId]: status }))
    })
    if (
      allInstalled &&
      Object.values(next).every(s => s.state === 'installed')
    ) {
      onDone(installed(next))
    } else {
      setPhase('done')
    }
  }

  const failed = Object.entries(statuses)
    .filter(([, status]) => status.state === 'failed')
    .map(([packId]) => packId)
  const finished = Object.values(statuses).filter(
    s => s.state === 'installed' || s.state === 'failed'
  ).length

  if (packs.isPending) {
    return (
      <PackGrid compact={compact}>
        {[0, 1, 2].map(i => (
          <Skeleton key={i} className="h-48 rounded-xl" />
        ))}
      </PackGrid>
    )
  }

  if (packs.isError) {
    return (
      <Empty className="py-10">
        <EmptyHeader>
          <EmptyTitle>{t('starterPacks.loadError')}</EmptyTitle>
          <EmptyDescription role="alert">
            {packs.error.message}
          </EmptyDescription>
        </EmptyHeader>
        <EmptyContent className="flex-row justify-center">
          <Button variant="outline" onClick={() => void packs.refetch()}>
            {t('starterPacks.retry')}
          </Button>
          <Button variant="ghost" onClick={onSkip}>
            {t('starterPacks.skip')}
          </Button>
        </EmptyContent>
      </Empty>
    )
  }

  if (available.length === 0) {
    return (
      <Empty className="py-10">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <PackageOpen />
          </EmptyMedia>
          <EmptyTitle>{t('starterPacks.emptyTitle')}</EmptyTitle>
          <EmptyDescription>
            {t('starterPacks.emptyDescription')}
          </EmptyDescription>
        </EmptyHeader>
        <EmptyContent>
          <Button onClick={() => onDone([])}>
            {t('starterPacks.continue')}
          </Button>
        </EmptyContent>
      </Empty>
    )
  }

  return (
    <div className="flex min-h-0 flex-col gap-4">
      <PackGrid compact={compact}>
        {available.map(pack => (
          <PackCard
            key={pack.id}
            pack={pack}
            selection={
              phase === 'choosing'
                ? {
                    checked: selected.includes(pack.id),
                    onCheckedChange: checked => toggle(pack.id, checked),
                    disabled: !pack.compatible,
                  }
                : undefined
            }
            status={statuses[pack.id]}
          />
        ))}
      </PackGrid>

      {limitHit && (
        <p role="status" className="text-sm text-muted-foreground">
          {t('starterPacks.limit', { max: MAX_STARTER_PACKS })}
        </p>
      )}

      <footer className="flex flex-wrap items-center justify-end gap-2">
        {onBack && (
          <Button
            variant="ghost"
            className="me-auto"
            disabled={phase === 'installing'}
            onClick={onBack}
          >
            {t('onboarding.back')}
          </Button>
        )}
        {phase === 'done' ? (
          <>
            <p role="status" className="me-auto text-sm text-destructive">
              {t('starterPacks.someFailed', { count: failed.length })}
            </p>
            <Button variant="outline" onClick={() => void install(failed)}>
              {t('starterPacks.retryFailed')}
            </Button>
            <Button onClick={() => onDone(installed(statuses))}>
              {t('starterPacks.continueWithout')}
            </Button>
          </>
        ) : (
          <>
            <Button
              variant="ghost"
              disabled={phase === 'installing'}
              onClick={onSkip}
            >
              {t('starterPacks.skip')}
            </Button>
            <Button
              disabled={selected.length === 0 || phase === 'installing'}
              onClick={() => void install(selected)}
            >
              {phase === 'installing' && <Spinner />}
              {phase === 'installing'
                ? t('starterPacks.installingProgress', {
                    current: Math.min(finished + 1, selected.length),
                    total: selected.length,
                  })
                : t('starterPacks.installSelected', {
                    count: selected.length,
                  })}
            </Button>
          </>
        )}
      </footer>
    </div>
  )
}

function PackGrid({
  children,
  compact,
}: {
  children: React.ReactNode
  compact?: boolean
}) {
  return (
    <div
      className={cn(
        'grid w-full grid-cols-1 gap-4',
        compact ? 'sm:grid-cols-2' : 'sm:grid-cols-2 xl:grid-cols-3'
      )}
    >
      {children}
    </div>
  )
}
