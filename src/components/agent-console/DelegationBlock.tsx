import { useTranslation } from 'react-i18next'
import { ChevronRight, MessageSquare } from 'lucide-react'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { useAgents } from '@/hooks/useAgents'
import { useDelegatedRuns } from '@/hooks/useRuns'
import { useRunDetail } from '@/hooks/useSnapshots'
import { decideApproval } from '@/lib/agent-actions'
import { displayAgent } from '@/lib/agent-identity'
import type { AgentInfo, RunSummary } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import type { UiMessage } from '@/types/agent'
import { AgentAvatar } from './AgentAvatar'
import { ChatMessageItem } from './ChatMessageItem'
import {
  delegationState,
  runsOfCall,
  splitChildMessages,
} from './delegation-model'
import { ToolCallCard } from './ToolCallCard'

type ToolCallMessage = Extract<UiMessage, { kind: 'toolCall' }>

/**
 * A `delegate_to_agent` call in the parent chat, read like a person in a
 * group chat: "<Agent> is working on it", its steps folded, its waiting
 * approvals open so the person decides right here (through the same
 * `decideApproval` as everywhere). Before the other agent starts (or for
 * another Nandi's agent), the call's own card.
 */
export function DelegationBlock({
  message,
  until,
}: {
  message: ToolCallMessage
  /** When the run's next delegation started, if any */
  until: number | null
}) {
  const { data } = useDelegatedRuns(message.runId)
  const children = runsOfCall(data ?? [], message.createdAt, until)
  if (children.length === 0) return <ToolCallCard message={message} />
  return (
    <div className="flex flex-col gap-3">
      {children.map(run => (
        <ChildRun key={run.id} run={run} call={message} />
      ))}
    </div>
  )
}

function ChildRun({ run, call }: { run: RunSummary; call: ToolCallMessage }) {
  const { t } = useTranslation()
  const agents = useAgents().data ?? []
  const { data: detail } = useRunDetail(run.id)
  const status = detail?.run.status ?? run.status
  const agent: AgentInfo =
    agents.find(a => a.id === run.agentId) ??
    displayAgent(run.agentId, detail?.agentName ?? run.agentId)
  const state = delegationState(status)
  const { waiting, steps } = splitChildMessages(detail?.messages ?? [])
  const decide = (approvalId: string, decision: 'approved' | 'rejected') =>
    decideApproval(run.agentId, approvalId, decision)

  return (
    <section
      aria-label={t(`liveWork.delegation.${state}`, { name: agent.name })}
      data-delegation-state={state}
      className={cn(
        'flex flex-col gap-3 rounded-lg border bg-card p-3',
        state === 'needsYou' && 'border-warning/40'
      )}
    >
      <header className="flex items-center gap-2">
        <AgentAvatar agent={agent} className="size-6" />
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {t(`liveWork.delegation.${state}`, { name: agent.name })}
        </span>
        <Button
          size="sm"
          variant="ghost"
          onClick={() =>
            useNavigationStore
              .getState()
              .navigate({ kind: 'live-run', runId: run.id })
          }
        >
          <MessageSquare />
          {t('liveWork.openChat')}
        </Button>
      </header>
      {waiting.map(message => (
        <ChatMessageItem
          key={message.id}
          message={message}
          agent={agent}
          onDecide={decide}
        />
      ))}
      <Collapsible>
        <CollapsibleTrigger className="group flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground">
          <ChevronRight
            aria-hidden
            className="size-3.5 transition-transform group-data-[state=open]:rotate-90 rtl:rotate-180"
          />
          {t('chat.showSteps', { count: steps.length + 1 })}
        </CollapsibleTrigger>
        <CollapsibleContent className="mt-3 flex flex-col gap-4">
          <ToolCallCard message={call} />
          {steps.map(message => (
            <ChatMessageItem
              key={message.id}
              message={message}
              agent={agent}
              onDecide={decide}
            />
          ))}
        </CollapsibleContent>
      </Collapsible>
    </section>
  )
}
