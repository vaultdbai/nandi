import still from '@/assets/brand/mascot/companion-bison-still.webp'
import { AgentAvatar } from '@/components/agent-console/AgentAvatar'
import { DEFAULT_AGENT, type AgentInfo } from '@/lib/bindings'
import { cn } from '@/lib/utils'

/** The sitting bison, Nandi's mark (decorative: the caller labels it) */
export function NandiMark({ className }: { className?: string }) {
  return (
    <img
      src={still}
      alt=""
      aria-hidden
      draggable={false}
      className={cn('object-contain', className ?? 'size-8')}
    />
  )
}

/** An agent's icon in a chat list: the bison for Nandi, its own for the rest */
export function AgentMark({
  agent,
  className,
}: {
  agent: AgentInfo
  className?: string
}) {
  return agent.id === DEFAULT_AGENT ? (
    <NandiMark className={className} />
  ) : (
    <AgentAvatar agent={agent} className={className} />
  )
}
