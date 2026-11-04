import { useTranslation } from 'react-i18next'
import { TriangleAlert } from 'lucide-react'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import { cn } from '@/lib/utils'
import type { AgentInfo, UiMessage } from '@/types/agent'
import { AgentAvatar } from './AgentAvatar'
import { ApprovalRequestCard } from './ApprovalRequestCard'
import { AttachmentChip } from './AttachmentChip'
import { MarkdownContent } from './MarkdownContent'
import { ToolCallCard } from './ToolCallCard'

interface ChatMessageItemProps {
  message: UiMessage
  agent: AgentInfo
  /** Omit for read-only transcripts */
  onDecide?: (
    approvalId: string,
    decision: 'approved' | 'rejected'
  ) => Promise<boolean>
}

/**
 * A single entry in the agent conversation log. Uses a flat, activity-feed
 * layout (avatar + author + timestamp) rather than chat bubbles.
 */
export function ChatMessageItem({
  message,
  agent,
  onDecide,
}: ChatMessageItemProps) {
  const { t, i18n } = useTranslation()
  const isUser = message.kind === 'user'
  const time = new Date(message.createdAt).toLocaleTimeString(i18n.language, {
    hour: 'numeric',
    minute: '2-digit',
  })

  return (
    <article
      aria-busy={message.pending || undefined}
      className={cn('flex gap-3', message.pending && 'opacity-60')}
    >
      {isUser ? (
        <Avatar className="size-9 rounded-lg">
          <AvatarFallback className="rounded-lg bg-primary text-xs font-medium text-primary-foreground">
            {t('agentConsole.chat.youInitial')}
          </AvatarFallback>
        </Avatar>
      ) : (
        <AgentAvatar agent={agent} />
      )}

      <div className="min-w-0 flex-1 space-y-1.5">
        <header className="flex items-baseline gap-2">
          <span className="text-sm font-medium text-foreground">
            {isUser ? t('agentConsole.chat.you') : agent.name}
          </span>
          <time
            dateTime={new Date(message.createdAt).toISOString()}
            className="text-xs text-muted-foreground tabular-nums"
          >
            {time}
          </time>
        </header>

        <MessageBody message={message} onDecide={onDecide} />
      </div>
    </article>
  )
}

function MessageBody({
  message,
  onDecide,
}: Pick<ChatMessageItemProps, 'message' | 'onDecide'>) {
  switch (message.kind) {
    case 'user':
      return (
        <div className="space-y-2">
          {message.content && (
            <p className="text-sm leading-relaxed whitespace-pre-wrap text-foreground/90">
              {message.content}
            </p>
          )}
          {message.attachments && message.attachments.length > 0 && (
            <div className="flex flex-wrap gap-1.5">
              {message.attachments.map(attachment => (
                <AttachmentChip key={attachment.id} attachment={attachment} />
              ))}
            </div>
          )}
        </div>
      )
    case 'agent':
      return <MarkdownContent content={message.content} />
    case 'toolCall':
      return <ToolCallCard message={message} />
    case 'approval':
      return <ApprovalRequestCard message={message} onDecide={onDecide} />
    case 'error':
      return (
        <p
          role="alert"
          className="flex items-start gap-2 rounded-md border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
        >
          <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
          <span className="min-w-0 break-words">{message.content}</span>
        </p>
      )
  }
}
