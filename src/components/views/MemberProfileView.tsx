import { useTranslation } from 'react-i18next'
import { ArrowLeft, Plug, UserRound } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { AskMemberComposer } from '@/components/network/AskMemberComposer'
import { MemberAvailability } from '@/components/network/MemberAvailability'
import { MemberAvatar } from '@/components/network/MemberAvatar'
import { MemberDiagnosis } from '@/components/network/MemberDiagnosis'
import { TrustPopover } from '@/components/network/TrustControls'
import { MemberAgentsPanel } from '@/components/team/MemberAgentsPanel'
import { MemberCapabilitiesPanel } from '@/components/team/MemberCapabilitiesPanel'
import { MemberDelegationsPanel } from '@/components/team/MemberDelegationsPanel'
import { MemberGatewayPanel } from '@/components/team/MemberGatewayPanel'
import { MemberTrustPanel } from '@/components/team/MemberTrustPanel'
import { useMemberProfile } from '@/hooks/useMemberProfile'
import { blockedAgentCount, isMemberBlocked } from '@/hooks/useTrust'
import type { MemberProfile } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'
import { useNavigationStore } from '@/store/navigation-store'
import { ViewHeader } from './ViewHeader'

export interface MemberProfileViewProps {
  instanceId: string
}

/**
 * A member's profile: who they are and our trust toward them (header), ask
 * them, then tabs for their gateway (NANDI.md), capabilities and agents,
 * recent delegations, and trust
 */
export function MemberProfileView({ instanceId }: MemberProfileViewProps) {
  const { t } = useTranslation()
  const profile = useMemberProfile(instanceId)
  const data = profile.data

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        title={data?.displayName ?? t('members.title')}
        subtitle={data?.handle ? `@${data.handle}` : t('network.unnamed')}
        leading={<UserRound className="size-4 text-muted-foreground" />}
        actions={
          <Button
            size="sm"
            variant="ghost"
            onClick={() =>
              useNavigationStore
                .getState()
                .navigate({ kind: 'network', tab: 'people' })
            }
          >
            <ArrowLeft className="rtl:rotate-180" />
            {t('members.backToNetwork')}
          </Button>
        }
      />
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto grid w-full max-w-6xl gap-4 p-6">
          {profile.isPending && (
            <>
              <Skeleton className="h-20" />
              <Skeleton className="h-64" />
            </>
          )}
          {profile.error && (
            <p className="text-sm text-destructive">
              {t('members.loadError', { error: profile.error.message })}
            </p>
          )}
          {data && <ProfileBody profile={data} />}
        </div>
      </ScrollArea>
    </div>
  )
}

function ProfileBody({ profile }: { profile: MemberProfile }) {
  const { t, i18n } = useTranslation()
  const { trust } = profile
  const presence = profile.online
    ? t('network.online')
    : profile.lastSeen === null
      ? t('network.offline')
      : t('network.lastSeen', {
          time: formatRelativeTime(profile.lastSeen, i18n.language),
        })

  return (
    <>
      <header className="flex flex-wrap items-center gap-4">
        <MemberAvatar
          name={profile.displayName}
          avatarUrl={profile.avatarUrl}
          online={profile.connected ? profile.online : undefined}
          className="size-14 text-base"
        />
        <div className="min-w-0">
          <h2 className="truncate text-xl font-semibold">
            {profile.displayName || t('network.unnamed')}
          </h2>
          <p className="text-sm text-muted-foreground">
            {profile.connected ? presence : t('members.notConnected')}
          </p>
          <MemberAvailability availability={profile.gateway?.availability} />
        </div>
        <div className="flex flex-wrap gap-1.5">
          {trust.addedAsTrustedPeer && (
            <Badge variant="secondary">{t('members.badge.trusted')}</Badge>
          )}
          {trust.delegationsPaused && (
            <Badge variant="outline">{t('members.badge.paused')}</Badge>
          )}
          {isMemberBlocked(trust) && (
            <Badge variant="destructive">{t('members.badge.blocked')}</Badge>
          )}
          {blockedAgentCount(trust) > 0 && (
            <Badge variant="destructive">
              {t('members.badge.blockedCount', {
                count: blockedAgentCount(trust),
              })}
            </Badge>
          )}
        </div>
        {profile.instanceId && (
          <div className="ms-auto">
            <TrustPopover instanceId={profile.instanceId} />
          </div>
        )}
      </header>

      {!profile.connected && (
        <Empty className="border">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <Plug />
            </EmptyMedia>
            <EmptyTitle>{t('members.notConnected')}</EmptyTitle>
            <EmptyDescription>
              {t('members.notConnectedDescription')}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )}

      {profile.connected && profile.instanceId && (
        <MemberDiagnosis profile={profile} />
      )}

      {profile.connected && profile.instanceId && (
        <AskMemberComposer
          instanceId={profile.instanceId}
          name={profile.displayName}
          online={profile.online}
          gateway={profile.gateway}
        />
      )}

      <Tabs defaultValue="gateway" className="min-w-0">
        <TabsList>
          <TabsTrigger value="gateway">{t('members.tab.gateway')}</TabsTrigger>
          <TabsTrigger value="capabilities">
            {t('members.tab.capabilities')}
          </TabsTrigger>
          <TabsTrigger value="delegations">
            {t('members.tab.delegations')}
          </TabsTrigger>
          <TabsTrigger value="trust">{t('members.tab.trust')}</TabsTrigger>
        </TabsList>
        <TabsContent value="gateway" className="flex flex-col gap-4">
          <MemberGatewayPanel profile={profile} />
        </TabsContent>
        <TabsContent value="capabilities" className="flex flex-col gap-4">
          <MemberCapabilitiesPanel profile={profile} />
          <MemberAgentsPanel profile={profile} />
        </TabsContent>
        <TabsContent value="delegations" className="flex flex-col gap-4">
          <MemberDelegationsPanel profile={profile} />
        </TabsContent>
        <TabsContent value="trust" className="flex flex-col gap-4">
          <MemberTrustPanel profile={profile} />
        </TabsContent>
      </Tabs>
    </>
  )
}
