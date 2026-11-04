import { useTranslation } from 'react-i18next'
import { BadgeCheck, ShieldQuestion } from 'lucide-react'
import { Badge } from '@/components/ui/badge'

/**
 * Whether the publisher's signature over this exact release verified
 * against their key in the registry.
 */
export function VerificationBadge({ verified }: { verified: boolean }) {
  const { t } = useTranslation()
  return verified ? (
    <Badge
      variant="outline"
      className="gap-1 border-success/40 text-success"
      title={t('marketplace.verifiedHint')}
    >
      <BadgeCheck className="size-3" />
      {t('marketplace.verified')}
    </Badge>
  ) : (
    <Badge
      variant="outline"
      className="gap-1 text-muted-foreground"
      title={t('marketplace.unverifiedHint')}
    >
      <ShieldQuestion className="size-3" />
      {t('marketplace.unverified')}
    </Badge>
  )
}
