import { useTranslation } from 'react-i18next'
import { Ban, Bot, CircleCheck } from 'lucide-react'
import { toast } from 'sonner'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { agentIdOf, useBlockAgentFromMember } from '@/hooks/useMemberProfile'
import type {
  MemberAgent,
  MemberAgentSource,
  MemberProfile,
} from '@/lib/bindings'

export interface MemberAgentsPanelProps {
  profile: MemberProfile
}

const SOURCES: MemberAgentSource[] = ['team', 'direct']

/** Member profile: the member's agents, by how they reach me (block / unblock per agent) */
export function MemberAgentsPanel({ profile }: MemberAgentsPanelProps) {
  const { t } = useTranslation()
  const block = useBlockAgentFromMember()
  const instanceId = profile.instanceId

  const toggle = (agent: MemberAgent) => {
    if (!instanceId) return
    const blocked = !agent.blocked
    block.mutate(
      { instanceId, agentId: agentIdOf(agent.card), blocked },
      {
        onSuccess: () =>
          toast.success(
            blocked
              ? t('members.agents.blocked', { name: agent.card.name })
              : t('members.agents.unblocked', { name: agent.card.name })
          ),
        onError: error =>
          toast.error(
            t('members.agents.blockFailed', { error: error.message })
          ),
      }
    )
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Bot className="size-4" aria-hidden />
          {t('members.agents.title')}
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {profile.agents.length === 0 && (
          <p className="text-sm text-muted-foreground">
            {t('members.agents.empty')}
          </p>
        )}
        {SOURCES.map(source => {
          const agents = profile.agents.filter(a => a.source === source)
          if (agents.length === 0) return null
          return (
            <section key={source} className="flex flex-col gap-2">
              <h3 className="text-xs font-medium text-muted-foreground uppercase">
                {t(`members.agents.source.${source}`)}
              </h3>
              {agents.map(agent => (
                <div
                  key={agent.card.id}
                  data-testid={`member-agent-${agent.card.id}`}
                  className="flex items-start justify-between gap-3 rounded-md border p-3"
                >
                  <div className="min-w-0">
                    <p className="flex items-center gap-2 text-sm font-medium">
                      <span className="truncate">{agent.card.name}</span>
                      {agent.blocked && (
                        <Badge variant="destructive">
                          {t('members.badge.blocked')}
                        </Badge>
                      )}
                      {agent.teamId && (
                        <Badge variant="outline" className="font-normal">
                          {agent.teamId}
                        </Badge>
                      )}
                    </p>
                    {agent.card.description && (
                      <p className="line-clamp-2 text-sm text-muted-foreground">
                        {agent.card.description}
                      </p>
                    )}
                    {agent.card.capabilities.length > 0 && (
                      <div className="mt-2 flex flex-wrap gap-1">
                        {agent.card.capabilities.map(capability => (
                          <Badge key={capability.name} variant="secondary">
                            {capability.name}
                          </Badge>
                        ))}
                      </div>
                    )}
                  </div>
                  <Button
                    size="sm"
                    variant={agent.blocked ? 'outline' : 'ghost'}
                    disabled={!instanceId || block.isPending}
                    onClick={() => toggle(agent)}
                  >
                    {agent.blocked ? <CircleCheck /> : <Ban />}
                    {agent.blocked
                      ? t('members.agents.unblock')
                      : t('members.agents.block')}
                  </Button>
                </div>
              ))}
            </section>
          )
        })}
      </CardContent>
    </Card>
  )
}
