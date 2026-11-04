import { useTranslation } from 'react-i18next'
import { CalendarClock } from 'lucide-react'
import type { Availability } from '@/lib/bindings'
import { useMemberName } from '@/hooks/useNetwork'
import { useOwnInstanceId } from '@/hooks/useRelay'
import { cn } from '@/lib/utils'
import { currentAvailability, formatUntil } from './availability-model'

/**
 * "Away until Mon, Nov 10 · Lena covers" ("You cover" when it's this
 * Nandi): a member's availability, small and calm. Nothing when they're
 * available.
 */
export function MemberAvailability({
  availability,
  className,
}: {
  availability: Availability | undefined
  className?: string
}) {
  const { t, i18n } = useTranslation()
  const current = currentAvailability(availability)
  const delegate = useMemberName(current?.delegateTo ?? null)
  const ownId = useOwnInstanceId()
  if (!current) return null
  const status =
    current.until === null
      ? t(`members.availability.${current.status}`)
      : t(`members.availability.${current.status}Until`, {
          time: formatUntil(current.until, i18n.language),
        })
  return (
    <span
      className={cn(
        'inline-flex items-center gap-1 text-xs text-muted-foreground',
        className
      )}
      data-testid="member-availability"
    >
      <CalendarClock className="size-3.5 shrink-0" aria-hidden />
      <span>
        {status}
        {delegate &&
          ` · ${
            current.delegateTo === ownId
              ? t('members.availability.coveredByYou')
              : t('members.availability.coveredBy', { name: delegate })
          }`}
      </span>
    </span>
  )
}
