/**
 * The fields to join a company relay, shared by onboarding and Network ›
 * Connection: its address, its join code, and "Paste invite", which fills
 * both from a teammate's invite. Pasting a whole invite message into the
 * address field fills both too. The join code is a password field and is
 * never shown again once typed.
 */
import { useTranslation } from 'react-i18next'
import { CheckCircle2, ClipboardPaste, XCircle } from 'lucide-react'
import { toast } from 'sonner'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { Spinner } from '@/components/ui/spinner'
import type { JoinMessage } from './join-model'
import { isInviteText, parseInvite } from './relay-invite'

export interface JoinDraft {
  url: string
  joinCode: string
}

export function JoinRelayFields({
  idPrefix,
  value,
  onChange,
  urlLabel,
  urlRef,
  onUrlKeyDown,
  invalid = false,
  describedBy,
}: {
  idPrefix: string
  value: JoinDraft
  onChange: (value: JoinDraft) => void
  urlLabel: string
  urlRef?: React.Ref<HTMLInputElement>
  onUrlKeyDown?: (event: React.KeyboardEvent<HTMLInputElement>) => void
  invalid?: boolean
  describedBy?: string
}) {
  const { t } = useTranslation()

  const fillFrom = (text: string): boolean => {
    const invite = parseInvite(text)
    if (!invite) return false
    onChange({ url: invite.url, joinCode: invite.joinCode ?? value.joinCode })
    return true
  }

  const pasteInvite = async () => {
    let text = ''
    try {
      text = (await navigator.clipboard?.readText()) ?? ''
    } catch {
      // Not allowed here: say how to paste instead
    }
    if (!fillFrom(text)) toast.error(t('relay.join.pasteFailed'))
  }

  return (
    <div className="grid gap-3">
      <div className="grid gap-1.5">
        <div className="flex items-center justify-between gap-2">
          <Label htmlFor={`${idPrefix}-url`}>{urlLabel}</Label>
          <Button
            type="button"
            size="sm"
            variant="ghost"
            onClick={() => void pasteInvite()}
          >
            <ClipboardPaste />
            {t('relay.join.pasteInvite')}
          </Button>
        </div>
        <Input
          id={`${idPrefix}-url`}
          ref={urlRef}
          dir="ltr"
          className="font-mono"
          spellCheck={false}
          placeholder="wss://relay.example.com/relay"
          value={value.url}
          aria-invalid={invalid || undefined}
          aria-describedby={describedBy}
          onChange={event => {
            const text = event.target.value
            if (!(isInviteText(text) && fillFrom(text)))
              onChange({ ...value, url: text })
          }}
          onKeyDown={onUrlKeyDown}
        />
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor={`${idPrefix}-code`}>{t('relay.join.codeLabel')}</Label>
        <Input
          id={`${idPrefix}-code`}
          type="password"
          dir="ltr"
          autoComplete="off"
          spellCheck={false}
          value={value.joinCode}
          onChange={event =>
            onChange({ ...value, joinCode: event.target.value })
          }
        />
        <p className="text-xs text-muted-foreground">
          {t('relay.join.codeHint')}
        </p>
      </div>
    </div>
  )
}

/** "Joining…", then the outcome in one calm line, and the relay's words */
export function JoinResult({
  pending,
  message,
}: {
  pending: boolean
  message: JoinMessage | null
}) {
  const { t } = useTranslation()
  if (pending)
    return (
      <p
        role="status"
        className="flex items-center gap-2 text-sm text-muted-foreground"
      >
        <Spinner />
        {t('relay.join.joining')}
      </p>
    )
  if (!message) return null
  if (message.ok)
    return (
      <p
        role="status"
        data-testid="relay-join-result"
        className="flex items-center gap-1 text-sm text-success"
      >
        <CheckCircle2 aria-hidden className="size-4 shrink-0" />
        {t(message.key, message.params)}
      </p>
    )
  return (
    <div role="alert" data-testid="relay-join-result" className="space-y-1">
      <p className="flex items-center gap-1 text-sm text-destructive">
        <XCircle aria-hidden className="size-4 shrink-0" />
        {t(message.key, message.params)}
      </p>
      {message.detail && (
        <p dir="auto" className="text-xs text-muted-foreground break-words">
          {message.detail}
        </p>
      )}
    </div>
  )
}
