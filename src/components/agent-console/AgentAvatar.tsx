import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import { cn } from '@/lib/utils'
import type { AgentInfo } from '@/types/agent'
import { getAgentAvatarClassName, getAgentInitials } from './agent-avatar'
import { AgentStatusDot } from './AgentStatusIndicator'

interface AgentAvatarProps {
  agent: AgentInfo
  /** Overlay the status dot on the avatar */
  showStatus?: boolean
  className?: string
}

export function AgentAvatar({
  agent,
  showStatus = false,
  className,
}: AgentAvatarProps) {
  return (
    <div className="relative shrink-0">
      <Avatar className={cn('size-9 rounded-lg', className)}>
        <AvatarFallback
          className={cn(
            'rounded-lg text-xs font-medium',
            getAgentAvatarClassName(agent.id)
          )}
        >
          {getAgentInitials(agent.name)}
        </AvatarFallback>
      </Avatar>
      {showStatus && (
        <AgentStatusDot
          status={agent.status}
          className="absolute -end-0.5 -bottom-0.5 rounded-full ring-2 ring-background"
        />
      )}
    </div>
  )
}
