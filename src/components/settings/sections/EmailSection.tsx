import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Mail, RefreshCw } from 'lucide-react'
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
import { Button } from '@/components/ui/button'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { EmailSignInStatus } from '@/components/email/EmailSignInStatus'
import { EmailReads } from '@/components/email/EmailReads'
import {
  useEmailAction,
  useEmailSignIn,
  useEmailStatus,
} from '@/hooks/useEmail'
import { formatRelativeTime } from '@/lib/format-time'
import type { EmailProviderStatus, EmailStatus } from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'
import {
  formatWatchList,
  intervalLabel,
  intervalOptions,
  mailboxCard,
  parseWatchList,
  type CardTone,
} from '../email/email-model'
import { SettingsGroup, SettingsSectionFrame } from '../SettingsFrame'

const TONE: Record<CardTone, string> = {
  muted: 'text-muted-foreground',
  ok: 'text-success',
  warning: 'text-warning',
  error: 'text-destructive',
}

/**
 * Settings › Email: connect Outlook or Gmail so the Chief of Staff keeps
 * My notes from it. Says what's read and what never happens, then each
 * mailbox's status and actions, then how often and whom to watch.
 */
export function EmailSection() {
  const { t } = useTranslation()
  const status = useEmailStatus()

  return (
    <SettingsSectionFrame
      title={t('settings.section.email')}
      description={t('email.description')}
    >
      <EmailReads />
      {status.isPending && <Spinner />}
      {status.error && (
        <p className="text-sm text-destructive">{String(status.error)}</p>
      )}
      {status.data && <EmailBody status={status.data} />}
    </SettingsSectionFrame>
  )
}

function EmailBody({ status }: { status: EmailStatus }) {
  const { t, i18n } = useTranslation()
  const anyConnected = status.providers.some(p => p.connected)
  return (
    <>
      <SettingsGroup title={t('email.mailboxes')}>
        {status.providers.map(provider => (
          <MailboxCardView
            key={provider.provider}
            status={provider}
            checking={status.checking}
          />
        ))}
        {anyConnected && status.nextCheckAt !== null && !status.checking && (
          <p className="text-xs text-muted-foreground">
            {t('email.nextCheck', {
              when: formatRelativeTime(status.nextCheckAt, i18n.language),
            })}
          </p>
        )}
      </SettingsGroup>
      <ScheduleSettings status={status} />
    </>
  )
}

function MailboxCardView({
  status,
  checking,
}: {
  status: EmailProviderStatus
  checking: boolean
}) {
  const { t, i18n } = useTranslation()
  const card = mailboxCard(status)
  const action = useEmailAction()
  const signIn = useEmailSignIn(status.provider)
  const waiting = signIn.state.phase === 'waiting'

  return (
    <div
      className="space-y-3 rounded-md border p-4"
      data-provider={status.provider}
    >
      <div className="flex flex-wrap items-center gap-2">
        <Mail className="size-4 text-muted-foreground" />
        <span className="font-medium">{card.name}</span>
        <span className={cn('text-sm', TONE[card.tone])}>
          {t(card.statusKey)}
        </span>
      </div>
      {card.account && (
        <p className="text-sm">
          {t('email.connectedAs', { account: card.account })}
        </p>
      )}
      {status.connected && (
        <p className="text-xs text-muted-foreground">
          {status.lastCheckedAt !== null
            ? t('email.lastChecked', {
                when: formatRelativeTime(status.lastCheckedAt, i18n.language),
                count: status.lastFound,
              })
            : t('email.notCheckedYet')}
        </p>
      )}
      {card.error && (
        <p className="text-xs text-muted-foreground">{card.error}</p>
      )}
      <EmailSignInStatus state={signIn.state} />
      <div className="flex flex-wrap gap-2">
        {card.canConnect && !waiting && (
          <Button size="sm" onClick={() => void signIn.start()}>
            {card.reconnect
              ? t('email.reconnect')
              : t('email.connect', { name: card.name })}
          </Button>
        )}
        {waiting && (
          <Button size="sm" variant="ghost" onClick={signIn.reset}>
            {t('email.cancel')}
          </Button>
        )}
        {card.canCheck && (
          <Button
            size="sm"
            variant="outline"
            disabled={checking || action.isPending}
            onClick={() => action.mutate({ kind: 'check' })}
          >
            {checking ? <Spinner /> : <RefreshCw />}
            {checking ? t('email.checking') : t('email.checkNow')}
          </Button>
        )}
        {card.canPause && (
          <Button
            size="sm"
            variant="outline"
            disabled={action.isPending}
            onClick={() =>
              action.mutate({
                kind: 'pause',
                provider: status.provider,
                paused: true,
              })
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
              action.mutate({
                kind: 'pause',
                provider: status.provider,
                paused: false,
              })
            }
          >
            {t('email.resume')}
          </Button>
        )}
        {card.canDisconnect && (
          <AlertDialog>
            <AlertDialogTrigger asChild>
              <Button size="sm" variant="ghost" disabled={action.isPending}>
                {t('email.disconnect')}
              </Button>
            </AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>
                  {t('email.disconnectTitle', { name: card.name })}
                </AlertDialogTitle>
                <AlertDialogDescription>
                  {t('email.disconnectBody')}
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>{t('email.cancel')}</AlertDialogCancel>
                <AlertDialogAction
                  onClick={() =>
                    action.mutate({
                      kind: 'disconnect',
                      provider: status.provider,
                    })
                  }
                >
                  {t('email.disconnect')}
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
        )}
      </div>
      {action.error && (
        <p className="text-sm text-destructive">{String(action.error)}</p>
      )}
    </div>
  )
}

function ScheduleSettings({ status }: { status: EmailStatus }) {
  const { t } = useTranslation()
  const action = useEmailAction()
  const [watch, setWatch] = useState(formatWatchList(status.watch))
  const intervals = intervalOptions(status.intervalMinutes)
  const save = (intervalMinutes: number, watchText: string) =>
    action.mutate({
      kind: 'settings',
      settings: { intervalMinutes, watch: parseWatchList(watchText) },
    })

  return (
    <SettingsGroup title={t('email.schedule')}>
      <label className="flex flex-wrap items-center gap-2 text-sm">
        {t('email.every')}
        <NativeSelect
          value={String(status.intervalMinutes)}
          onChange={event => save(Number(event.target.value), watch)}
          aria-label={t('email.every')}
        >
          {intervals.map(minutes => (
            <NativeSelectOption key={minutes} value={String(minutes)}>
              {intervalLabel(t, minutes)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </label>
      <p className="text-xs text-muted-foreground">{t('email.oneRun')}</p>
      <div className="space-y-2">
        <label htmlFor="email-watch" className="text-sm font-medium">
          {t('email.watch')}
        </label>
        <p className="text-xs text-muted-foreground">{t('email.watchHelp')}</p>
        <Textarea
          id="email-watch"
          dir="ltr"
          rows={3}
          value={watch}
          placeholder={t('email.watchPlaceholder')}
          onChange={event => setWatch(event.target.value)}
        />
        <Button
          size="sm"
          variant="outline"
          disabled={action.isPending || watch === formatWatchList(status.watch)}
          onClick={() => save(status.intervalMinutes, watch)}
        >
          {t('email.saveWatch')}
        </Button>
        {action.error && (
          <p className="text-sm text-destructive">{String(action.error)}</p>
        )}
      </div>
    </SettingsGroup>
  )
}
