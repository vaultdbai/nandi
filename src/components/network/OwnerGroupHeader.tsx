import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { UserRound } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { useNavigationStore } from '@/store/navigation-store'
import { MemberAvatar } from './MemberAvatar'

export interface OwnerGroupHeaderProps {
  /** Owner handle (user slug); empty when the cards don't name one */
  handle: string
  displayName: string
  count: number
  /** Their instance, when known: enables "View profile" */
  instanceId?: string | null
  /** Extra controls on the end side (e.g. `TrustControls`) */
  children?: ReactNode
}

/** The heading of one owner's group in the shared grids */
export function OwnerGroupHeader({
  handle,
  displayName,
  count,
  instanceId,
  children,
}: OwnerGroupHeaderProps) {
  const { t } = useTranslation()
  const name = displayName || handle || t('shared.unknownOwner')
  return (
    <div
      data-testid={`owner-group-${handle || instanceId || 'unknown'}`}
      className="flex flex-wrap items-center gap-3"
    >
      <MemberAvatar name={name} className="size-8" />
      <div className="min-w-0">
        <h2 className="truncate text-sm font-semibold">{name}</h2>
        <p className="truncate text-xs text-muted-foreground" dir="auto">
          {handle ? `@${handle} · ` : ''}
          {t('network.groupCount', { count })}
        </p>
      </div>
      <div className="ms-auto flex flex-wrap items-center gap-3">
        {children}
        {instanceId && (
          <Button
            size="sm"
            variant="ghost"
            onClick={() =>
              useNavigationStore
                .getState()
                .navigate({ kind: 'member-profile', instanceId })
            }
          >
            <UserRound />
            {t('network.viewProfile')}
          </Button>
        )}
      </div>
    </div>
  )
}
