import { useTranslation } from 'react-i18next'
import { ShieldCheck } from 'lucide-react'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { TrustControls } from '@/components/network/TrustControls'
import type { MemberProfile } from '@/lib/bindings'

export interface MemberTrustPanelProps {
  profile: MemberProfile
}

/** Member profile: trusted peer, paused delegations */
export function MemberTrustPanel({ profile }: MemberTrustPanelProps) {
  const { t } = useTranslation()
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ShieldCheck className="size-4" aria-hidden />
          {t('members.trust.title')}
        </CardTitle>
        <CardDescription>{t('members.trust.description')}</CardDescription>
      </CardHeader>
      <CardContent>
        {profile.instanceId ? (
          <TrustControls
            instanceId={profile.instanceId}
            trust={profile.trust}
            showHints
          />
        ) : (
          <p className="text-sm text-muted-foreground">
            {t('members.trust.unavailable')}
          </p>
        )}
      </CardContent>
    </Card>
  )
}
