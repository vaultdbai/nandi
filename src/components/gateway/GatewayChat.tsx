import { useState, type FormEvent, type KeyboardEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { MessageSquare, Send } from 'lucide-react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Spinner } from '@/components/ui/spinner'
import { WorkingDots } from '@/components/agent-console/WorkingDots'
import { Textarea } from '@/components/ui/textarea'
import { rawError } from '@/lib/friendly-error'
import { useGatewayChat } from '@/hooks/useGateway'
import { cn } from '@/lib/utils'

interface ChatMessage {
  id: number
  from: 'user' | 'gateway'
  text: string
}

/**
 * Talk to the gateway agent as a caller would: it answers from NANDI.md,
 * the capability catalog, and its queue, and never runs work itself.
 * The conversation is local to the panel; each message is answered on
 * its own (`gateway_chat`).
 */
export function GatewayChat() {
  const { t } = useTranslation()
  const chat = useGatewayChat()
  const [messages, setMessages] = useState<ChatMessage[]>([])
  const [draft, setDraft] = useState('')
  const [nextId, setNextId] = useState(0)

  const send = () => {
    const message = draft.trim()
    if (!message || chat.isPending) return
    const userId = nextId
    setNextId(userId + 2)
    setMessages(current => [
      ...current,
      { id: userId, from: 'user', text: message },
    ])
    setDraft('')
    chat.mutate(message, {
      onSuccess: reply =>
        setMessages(current => [
          ...current,
          { id: userId + 1, from: 'gateway', text: reply },
        ]),
    })
  }

  const handleSubmit = (event: FormEvent) => {
    event.preventDefault()
    send()
  }

  const handleKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault()
      send()
    }
  }

  return (
    <Card data-testid="gateway-chat">
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <MessageSquare className="size-4" />
          {t('gateway.chat.title')}
        </CardTitle>
        <CardDescription>{t('gateway.chat.description')}</CardDescription>
      </CardHeader>
      <CardContent className="grid gap-3">
        <ol
          className="flex max-h-96 flex-col gap-2 overflow-y-auto"
          aria-label={t('gateway.chat.messages')}
          aria-live="polite"
          data-testid="gateway-chat-messages"
        >
          {messages.length === 0 && (
            <li className="text-sm text-muted-foreground">
              {t('gateway.chat.empty')}
            </li>
          )}
          {messages.map(message => (
            <li
              key={message.id}
              data-from={message.from}
              className={cn(
                'max-w-[85%] rounded-lg px-3 py-2 text-sm whitespace-pre-wrap',
                message.from === 'user'
                  ? 'self-end bg-primary text-primary-foreground'
                  : 'self-start bg-muted'
              )}
            >
              <span className="sr-only">
                {t(`gateway.chat.from.${message.from}`)}:{' '}
              </span>
              {message.text}
            </li>
          ))}
          {chat.isPending && (
            <li
              role="status"
              className="self-start text-sm text-muted-foreground"
            >
              <WorkingDots label={t('gateway.chat.thinking')} />
            </li>
          )}
        </ol>
        {chat.isError && (
          <Alert variant="destructive">
            <AlertTitle>{t('gateway.chat.failed')}</AlertTitle>
            <AlertDescription>{rawError(chat.error)}</AlertDescription>
          </Alert>
        )}
        <form className="grid gap-2" onSubmit={handleSubmit}>
          <Textarea
            aria-label={t('gateway.chat.placeholder')}
            placeholder={t('gateway.chat.placeholder')}
            value={draft}
            onChange={e => setDraft(e.target.value)}
            onKeyDown={handleKeyDown}
          />
          <div className="flex justify-end">
            <Button type="submit" disabled={!draft.trim() || chat.isPending}>
              {chat.isPending ? <Spinner /> : <Send />}
              {t('gateway.chat.send')}
            </Button>
          </div>
        </form>
      </CardContent>
    </Card>
  )
}
