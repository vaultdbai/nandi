import { useTranslation } from 'react-i18next'
import { Sparkles } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import type { MemberProfile } from '@/lib/bindings'

export interface MemberCapabilitiesPanelProps {
  profile: MemberProfile
}

/** Member profile: the capabilities the member serves */
export function MemberCapabilitiesPanel({
  profile,
}: MemberCapabilitiesPanelProps) {
  const { t } = useTranslation()
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Sparkles className="size-4" aria-hidden />
          {t('members.capabilities.title')}
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-wrap gap-1.5">
        {profile.capabilities.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            {t('members.capabilities.empty')}
          </p>
        ) : (
          profile.capabilities.map(capability => (
            <Badge key={capability} variant="secondary">
              {capability}
            </Badge>
          ))
        )}
      </CardContent>
    </Card>
  )
}
