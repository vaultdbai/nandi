import { useEffect, useRef } from 'react'
import { useTranslation } from 'react-i18next'
import { ChevronRight } from 'lucide-react'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { ScrollArea } from '@/components/ui/scroll-area'
import { cn } from '@/lib/utils'
import type { AgentInfo, UiMessage } from '@/types/agent'
import { ChatMessageItem } from './ChatMessageItem'
import { WorkingRow } from './WorkingDots'
import { DelegationBlock } from './DelegationBlock'
import { PersonAskCard } from './PersonAskCard'
import { delegationWindows } from './delegation-model'
import { hiddenPlanCalls } from './run-plan-model'
import { foldSteps, type ChatItem } from './steps-model'
import { showsWorking, turnStartedAt } from './working'

interface MessageListProps {
  messages: UiMessage[]
  /** Identity shown for non-user messages */
  assistant: AgentInfo
  /** Omit for read-only transcripts */
  onDecide?: (
    approvalId: string,
    decision: 'approved' | 'rejected'
  ) => Promise<boolean>
  /** Scroll to and briefly highlight this message */
  focusMessageId?: string
  /** Rendered when there are no messages */
  empty?: React.ReactNode
  /** The assistant is on a turn: dots show until its reply streams in */
  working?: boolean
  /** Fold each stretch of tool calls under "Show steps" (a live chat) */
  quietSteps?: boolean
}

/**
 * Scrollable conversation log shared by agent chat, Studio, and run
 * transcripts. Follows new messages and streamed text unless a specific
 * message is focused.
 */
export function MessageList({
  messages,
  assistant,
  onDecide,
  focusMessageId,
  empty,
  working = false,
  quietSteps = false,
}: MessageListProps) {
  const { t } = useTranslation()
  const endRef = useRef<HTMLDivElement>(null)
  const focusRef = useRef<HTMLDivElement>(null)

  const lastMessage = messages.at(-1)
  const dots = showsWorking(messages, working)
  const scrollKey = `${messages.length}:${lastMessage?.kind === 'agent' ? lastMessage.content.length : 0}:${dots}`
  const hasFocusTarget = messages.some(m => m.id === focusMessageId)
  // One checklist card per run stands for its plan tool calls
  const hiddenPlan = hiddenPlanCalls(messages)

  useEffect(() => {
    if (hasFocusTarget) {
      focusRef.current?.scrollIntoView?.({ block: 'center' })
    } else {
      endRef.current?.scrollIntoView?.({ block: 'end' })
    }
  }, [scrollKey, hasFocusTarget, focusMessageId])

  if (messages.length === 0) {
    return <div className="min-h-0 flex-1">{empty}</div>
  }

  const visible = messages
    .filter(m => m.kind !== 'agent' || m.content.length > 0)
    .filter(m => !hiddenPlan.has(m.id))
  const items: ChatItem[] = quietSteps
    ? foldSteps(visible)
    : visible.map(message => ({ kind: 'message', message }))

  // Each delegation's window: the runs it started, until the next one
  const delegations = delegationWindows(visible)

  const renderMessage = (message: UiMessage) => {
    const focused = message.id === focusMessageId
    return (
      <div
        key={message.id}
        ref={focused ? focusRef : undefined}
        data-focused={focused || undefined}
        className={cn(
          'rounded-lg transition-shadow',
          focused &&
            'ring-2 ring-primary/40 ring-offset-4 ring-offset-background'
        )}
      >
        {message.kind === 'toolCall' && delegations.has(message.id) ? (
          <DelegationBlock
            message={message}
            until={delegations.get(message.id) ?? null}
          />
        ) : (
          <ChatMessageItem
            message={message}
            agent={assistant}
            onDecide={onDecide}
          />
        )}
        {message.kind === 'user' && message.asked.length > 0 && (
          <PersonAskCard asks={message.asked} />
        )}
      </div>
    )
  }

  return (
    <ScrollArea className="min-h-0 flex-1">
      {/* The app turns selection off; chat text can be selected and copied */}
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-6 px-6 py-6 select-text">
        {items.map(item => {
          if (item.kind === 'message') return renderMessage(item.message)
          const open = item.messages.some(m => m.id === focusMessageId)
          return (
            <Collapsible key={item.id} defaultOpen={open}>
              <CollapsibleTrigger className="group flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground">
                <ChevronRight
                  aria-hidden
                  className="size-3.5 transition-transform group-data-[state=open]:rotate-90 rtl:rotate-180"
                />
                {t('chat.showSteps', { count: item.messages.length })}
              </CollapsibleTrigger>
              <CollapsibleContent className="mt-3 flex flex-col gap-6">
                {item.messages.map(renderMessage)}
              </CollapsibleContent>
            </Collapsible>
          )
        })}
        {dots && (
          <WorkingRow agent={assistant} since={turnStartedAt(messages)} />
        )}
        <div ref={endRef} />
      </div>
    </ScrollArea>
  )
}
