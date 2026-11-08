import { useEffect, useRef } from 'react'
import { ScrollArea } from '@/components/ui/scroll-area'
import { cn } from '@/lib/utils'
import type { AgentInfo, UiMessage } from '@/types/agent'
import { ChatMessageItem } from './ChatMessageItem'
import { WorkingRow } from './WorkingDots'
import { hiddenPlanCalls } from './run-plan-model'
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
}: MessageListProps) {
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

  return (
    <ScrollArea className="min-h-0 flex-1">
      {/* The app turns selection off; chat text can be selected and copied */}
      <div className="mx-auto flex w-full max-w-3xl flex-col gap-6 px-6 py-6 select-text">
        {messages
          .filter(m => m.kind !== 'agent' || m.content.length > 0)
          .filter(m => !hiddenPlan.has(m.id))
          .map(message => {
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
                <ChatMessageItem
                  message={message}
                  agent={assistant}
                  onDecide={onDecide}
                />
              </div>
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
