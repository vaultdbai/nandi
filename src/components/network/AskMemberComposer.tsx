import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { MessageSquare, Send } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Label } from '@/components/ui/label'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { useDelegateTask } from '@/hooks/useDelegation'
import { rawError } from '@/lib/friendly-error'
import { useRelayConnected } from '@/hooks/useRelay'
import type { GatewayCard } from '@/lib/bindings'
import { errorKind, isRelayUnavailable } from '@/lib/friendly-error'
import { plainError } from '@/lib/toast-error'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import { askAvailability, gatewayRequest } from './ask-model'
import { RelayConnectNotice } from './RelayConnectNotice'

export interface AskMemberComposerProps {
  instanceId: string
  name: string
  online: boolean
  gateway: GatewayCard | null
  /** Without the heading and description (popovers, cards) */
  compact?: boolean
  /** The request was sent (before navigating to its thread) */
  onSent?: (taskId: string) => void
}

/**
 * "Ask <member>": a plain-words request to their gateway, which routes it
 * to one of their agents. Sending opens the task's thread (state changes,
 * the routing reason, and the result). Disabled, with the reason, while
 * the member is offline or their gateway isn't accepting requests; with
 * no relay connected, "Connect to your company's relay" instead.
 */
export function AskMemberComposer({
  instanceId,
  name,
  online,
  gateway,
  compact = false,
  onSent,
}: AskMemberComposerProps) {
  const { t } = useTranslation()
  const delegate = useDelegateTask()
  const [text, setText] = useState('')
  const [relayRefused, setRelayRefused] = useState(false)
  const needsRelay = useRelayConnected() === false || relayRefused
  const availability = askAvailability({ online, gateway })
  const available = availability === 'available' && !needsRelay
  const canSend = available && text.trim() !== '' && !delegate.isPending
  const capabilities = gateway?.capabilities ?? []
  const id = `ask-${instanceId}`

  const send = async () => {
    if (!canSend) return
    try {
      const taskId = await delegate.mutateAsync(
        gatewayRequest(instanceId, text)
      )
      setText('')
      toast.success(t('members.ask.sent', { name }))
      onSent?.(taskId)
      useNavigationStore
        .getState()
        .navigate({ kind: 'delegation-thread', taskId })
    } catch (error) {
      if (isRelayUnavailable(error)) setRelayRefused(true)
      // Past the public relay's messages a day: calmly, with the reset
      else if (errorKind(error) === 'dailyLimit') toast(plainError(error))
      else toast.error(t('members.ask.failed', { error: rawError(error) }))
    }
  }

  return (
    <form
      className={cn('flex flex-col gap-3', !compact && 'rounded-lg border p-4')}
      data-testid="ask-member"
      onSubmit={event => {
        event.preventDefault()
        void send()
      }}
    >
      {!compact && (
        <div className="space-y-1">
          <h3 className="flex items-center gap-2 font-semibold">
            <MessageSquare className="size-4" aria-hidden />
            {t('members.ask.title', { name })}
          </h3>
          <p className="text-sm text-muted-foreground">
            {t('members.ask.description', { name })}
          </p>
        </div>
      )}
      <Label htmlFor={id} className={cn(!compact && 'sr-only')}>
        {t('members.ask.title', { name })}
      </Label>
      <Textarea
        id={id}
        dir="auto"
        value={text}
        disabled={!available || delegate.isPending}
        placeholder={t('members.ask.placeholder')}
        onChange={event => setText(event.target.value)}
        onKeyDown={event => {
          if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
            event.preventDefault()
            void send()
          }
        }}
      />
      {available && capabilities.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <span className="text-xs text-muted-foreground">
            {t('members.ask.examples')}
          </span>
          {capabilities.map(capability => (
            <Badge key={capability} variant="outline" asChild>
              <button
                type="button"
                className="cursor-pointer"
                onClick={() =>
                  setText(t('members.ask.chipPrompt', { capability }))
                }
              >
                {capability}
              </button>
            </Badge>
          ))}
        </div>
      )}
      {needsRelay && <RelayConnectNotice variant="inline" />}
      <div className="flex flex-wrap items-center justify-end gap-3">
        {!available && !needsRelay && (
          <p
            className="me-auto text-sm text-muted-foreground"
            data-testid="ask-unavailable"
          >
            {t(`members.ask.unavailable.${availability}`, { name })}
          </p>
        )}
        <Button type="submit" size="sm" disabled={!canSend}>
          {delegate.isPending ? (
            <Spinner />
          ) : (
            <Send className="rtl:-scale-x-100" />
          )}
          {t('members.ask.send')}
        </Button>
      </div>
    </form>
  )
}
