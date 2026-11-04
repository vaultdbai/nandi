import { useTranslation } from 'react-i18next'
import { Avatar, AvatarFallback, AvatarImage } from '@/components/ui/avatar'
import { initialsOf } from '@/hooks/useNetwork'
import { cn } from '@/lib/utils'

export interface MemberAvatarProps {
  name: string
  avatarUrl?: string | null
  /** Draws a presence dot; `undefined` draws none */
  online?: boolean
  className?: string
}

/** A member's avatar (initials when there's no picture), with presence */
export function MemberAvatar({
  name,
  avatarUrl,
  online,
  className,
}: MemberAvatarProps) {
  const { t } = useTranslation()
  return (
    <span className="relative inline-flex shrink-0">
      <Avatar className={cn('size-9', className)}>
        {avatarUrl && <AvatarImage src={avatarUrl} alt="" />}
        <AvatarFallback className="text-xs font-medium">
          {initialsOf(name)}
        </AvatarFallback>
      </Avatar>
      {online !== undefined && (
        <span
          role="status"
          aria-label={online ? t('network.online') : t('network.offline')}
          className={cn(
            'absolute -end-0.5 -bottom-0.5 size-2.5 rounded-full border-2 border-background',
            online ? 'bg-success' : 'bg-muted-foreground/40'
          )}
        />
      )}
    </span>
  )
}
