import { useTranslation } from 'react-i18next'
import { Network } from 'lucide-react'
import { MarkdownContent } from '@/components/agent-console/MarkdownContent'
import { Badge } from '@/components/ui/badge'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import type { MemberProfile } from '@/lib/bindings'

export interface MemberGatewayPanelProps {
  profile: MemberProfile
}

/** Member profile: the member's gateway agent (their NANDI.md, when they share it) */
export function MemberGatewayPanel({ profile }: MemberGatewayPanelProps) {
  const { t } = useTranslation()
  const { gateway } = profile
  if (!gateway) return null

  return (
    <Card data-testid="member-gateway">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Network className="size-4" aria-hidden />
          {t('members.gateway.title')}
        </CardTitle>
        {gateway.description && (
          <CardDescription>{gateway.description}</CardDescription>
        )}
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        <div className="flex flex-wrap gap-1.5">
          <Badge variant={gateway.acceptsDelegation ? 'secondary' : 'outline'}>
            {gateway.acceptsDelegation
              ? t('members.gateway.accepting')
              : t('members.gateway.notAccepting')}
          </Badge>
          {gateway.acceptsDelegation && gateway.directDelegation === false && (
            <Badge variant="outline" data-testid="member-gateway-direct">
              {t('members.gateway.gatewayOnly')}
            </Badge>
          )}
          <Badge variant="outline">
            {t('members.gateway.load', {
              current: gateway.currentLoad,
              max: gateway.maxConcurrent,
            })}
          </Badge>
          {gateway.federationEnabled && (
            <Badge variant="outline">{t('members.gateway.federated')}</Badge>
          )}
        </div>
        {(gateway.responsibilities ?? []).length > 0 && (
          <div className="grid gap-1" data-testid="member-responsibilities">
            <h3 className="text-xs font-medium text-muted-foreground">
              {t('members.gateway.responsibilities')}
            </h3>
            <ul className="flex flex-wrap gap-1.5">
              {(gateway.responsibilities ?? []).map(item => (
                <li key={item}>
                  <Badge variant="secondary" dir="auto">
                    {item}
                  </Badge>
                </li>
              ))}
            </ul>
          </div>
        )}
        {gateway.body ? (
          <div className="text-sm">
            <MarkdownContent content={gateway.body} />
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">
            {t('members.gateway.noBody')}
          </p>
        )}
      </CardContent>
    </Card>
  )
}
