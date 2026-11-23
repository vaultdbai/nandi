import { useTranslation } from 'react-i18next'
import { Mail } from 'lucide-react'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { EmailSignInStatus } from '@/components/email/EmailSignInStatus'
import {
  backfillLine,
  mailboxCard,
  mailboxOf,
  type CardTone,
} from '@/components/settings/email/email-model'
import { useEmailAction, useEmailSignIn } from '@/hooks/useEmail'
import { formatRelativeTime } from '@/lib/format-time'
import type { MailboxStatus } from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'

const TONE: Record<CardTone, string> = {
  muted: 'text-muted-foreground',
  ok: 'text-success',
  warning: 'text-warning',
  error: 'text-destructive',
}

/**
 * One connected mailbox in Settings › Email: its provider, address and
 * status, when it was last read, the background read of older mail, and
 * its own Pause or Resume, Reconnect and Sign out. The identity mailbox is
 * marked, and signing it out says what that does.
 */
export function MailboxRow({ status }: { status: MailboxStatus }) {
  const { t, i18n } = useTranslation()
  const card = mailboxCard(status)
  const mailbox = mailboxOf(status)
  const action = useEmailAction()
  const signIn = useEmailSignIn(status.provider)
  const waiting = signIn.state.phase === 'waiting'

  return (
    <div
      className="space-y-3 rounded-md border p-4"
      data-provider={status.provider}
      data-mailbox={status.email}
    >
      <div className="flex flex-wrap items-center gap-2">
        <Mail className="size-4 text-muted-foreground" />
        <span className="font-medium">{card.name}</span>
        <span className={cn('text-sm', TONE[card.tone])}>
          {t(card.statusKey)}
        </span>
        {status.isIdentity && (
          <Badge variant="secondary" title={t('email.identityHint')}>
            {t('email.identity')}
          </Badge>
        )}
      </div>
      <p className="text-sm">{card.account}</p>
      <p className="text-xs text-muted-foreground">
        {status.lastCheckedAt !== null
          ? t('email.lastChecked', {
              when: formatRelativeTime(status.lastCheckedAt, i18n.language),
              count: status.lastFound,
            })
          : t('email.notCheckedYet')}
      </p>
      <BackfillLineView status={status} />
      {card.error && (
        <p className="text-xs text-muted-foreground">{card.error}</p>
      )}
      <EmailSignInStatus state={signIn.state} />
      <div className="flex flex-wrap gap-2">
        {card.reconnect && !waiting && (
          <Button size="sm" onClick={() => void signIn.start()}>
            {t('email.reconnect')}
          </Button>
        )}
        {waiting && (
          <Button size="sm" variant="ghost" onClick={signIn.reset}>
            {t('email.cancel')}
          </Button>
        )}
        {card.canPause && (
          <Button
            size="sm"
            variant="outline"
            disabled={action.isPending}
            onClick={() =>
              action.mutate({ kind: 'pause', mailbox, paused: true })
            }
          >
            {t('email.pause')}
          </Button>
        )}
        {card.canResume && (
          <Button
            size="sm"
            variant="outline"
            disabled={action.isPending}
            onClick={() =>
              action.mutate({ kind: 'pause', mailbox, paused: false })
            }
          >
            {t('email.resume')}
          </Button>
        )}
        <AlertDialog>
          <AlertDialogTrigger asChild>
            <Button size="sm" variant="ghost" disabled={action.isPending}>
              {t('email.disconnect')}
            </Button>
          </AlertDialogTrigger>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>
                {t('email.disconnectTitle', { name: status.email })}
              </AlertDialogTitle>
              <AlertDialogDescription>
                {t('email.disconnectBody')}
                {status.isIdentity && ` ${t('email.disconnectIdentity')}`}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>{t('email.cancel')}</AlertDialogCancel>
              <AlertDialogAction
                onClick={() => action.mutate({ kind: 'disconnect', mailbox })}
              >
                {t('email.disconnect')}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      </div>
      {action.error && (
        <p className="text-sm text-destructive">{String(action.error)}</p>
      )}
    </div>
  )
}

/** The background read of older mail: where it is, with its controls */
function BackfillLineView({ status }: { status: MailboxStatus }) {
  const { t, i18n } = useTranslation()
  const action = useEmailAction()
  const line = backfillLine(status.backfill)
  const mailbox = mailboxOf(status)
  if (line.key === null) return null
  const when =
    line.when === null
      ? ''
      : new Intl.DateTimeFormat(i18n.language, {
          month: 'long',
          year: 'numeric',
        }).format(new Date(line.when))
  const act = (kind: 'pause' | 'resume' | 'stop') =>
    action.mutate({ kind: 'backfill', mailbox, action: kind })
  return (
    <div
      className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground"
      data-testid="email-backfill"
    >
      <span>{t(line.key, { when, count: line.count })}</span>
      {line.canPause && (
        <Button
          size="sm"
          variant="ghost"
          className="h-7"
          onClick={() => act('pause')}
        >
          {t('email.pause')}
        </Button>
      )}
      {line.canResume && (
        <Button
          size="sm"
          variant="ghost"
          className="h-7"
          onClick={() => act('resume')}
        >
          {t('email.resume')}
        </Button>
      )}
      {line.canStop && (
        <Button
          size="sm"
          variant="ghost"
          className="h-7"
          onClick={() => act('stop')}
        >
          {t('email.backfill.stop')}
        </Button>
      )}
    </div>
  )
}
