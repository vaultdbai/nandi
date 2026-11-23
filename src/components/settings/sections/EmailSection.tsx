import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { RefreshCw } from 'lucide-react'
import { Button } from '@/components/ui/button'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { Spinner } from '@/components/ui/spinner'
import { Textarea } from '@/components/ui/textarea'
import { ConnectMailbox } from '@/components/email/ConnectMailbox'
import { BackfillChoice } from '@/components/email/BackfillChoice'
import { EmailReads } from '@/components/email/EmailReads'
import { MailboxRow } from '@/components/email/MailboxRow'
import { useEmailAction, useEmailStatus } from '@/hooks/useEmail'
import { formatRelativeTime } from '@/lib/format-time'
import type { EmailStatus } from '@/lib/tauri-bindings'
import {
  addableProviders,
  canCheckNow,
  formatWatchList,
  intervalLabel,
  intervalOptions,
  parseWatchList,
  PROVIDER_NAMES,
  providerStatusKey,
} from '../email/email-model'
import { SettingsGroup, SettingsSectionFrame } from '../SettingsFrame'

/**
 * Settings › Email: connect Outlook or Gmail so Nandi keeps
 * My notes from it, as many mailboxes as the person has. Says what's read
 * and what never happens, then one row per mailbox (status and its own
 * actions) with Add another account, then how often and whom to watch.
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
  const action = useEmailAction()
  const anyConnected = status.mailboxes.length > 0
  const unavailable = status.providers.filter(p => !p.available)
  return (
    <>
      <SettingsGroup title={t('email.mailboxes')}>
        {!anyConnected && (
          <p className="text-sm text-muted-foreground">
            {t('email.noMailboxes')}
          </p>
        )}
        {status.mailboxes.map(mailbox => (
          <MailboxRow
            key={`${mailbox.provider}:${mailbox.email}`}
            status={mailbox}
          />
        ))}
        {unavailable.map(provider => (
          <p
            key={provider.provider}
            className="text-sm text-muted-foreground"
            data-provider={provider.provider}
          >
            {PROVIDER_NAMES[provider.provider]}:{' '}
            {t(providerStatusKey(provider))}
          </p>
        ))}
        <div className="flex flex-wrap gap-2">
          {addableProviders(status.providers).map(provider => (
            <ConnectMailbox
              key={provider}
              provider={provider}
              variant="outline"
              another={anyConnected}
            />
          ))}
        </div>
        {canCheckNow(status.mailboxes) && (
          <div className="flex flex-wrap gap-2">
            <Button
              size="sm"
              variant="outline"
              disabled={status.checking || action.isPending}
              onClick={() => action.mutate({ kind: 'check' })}
            >
              {status.checking ? <Spinner /> : <RefreshCw />}
              {status.checking ? t('email.checking') : t('email.checkNow')}
            </Button>
            {/* The resync: the last few days again, the cursors where they are */}
            <Button
              size="sm"
              variant="ghost"
              title={t('email.readAgainHint')}
              disabled={status.checking || action.isPending}
              onClick={() => action.mutate({ kind: 'again' })}
            >
              {t('email.readAgain')}
            </Button>
          </div>
        )}
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

function ScheduleSettings({ status }: { status: EmailStatus }) {
  const { t } = useTranslation()
  const action = useEmailAction()
  const [watch, setWatch] = useState(formatWatchList(status.watch))
  const intervals = intervalOptions(status.intervalMinutes)
  const save = (intervalMinutes: number, watchText: string) =>
    action.mutate({
      kind: 'settings',
      settings: {
        intervalMinutes,
        watch: parseWatchList(watchText),
        backfill: status.backfillRange,
      },
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
      <BackfillChoice
        value={status.backfillRange}
        onChange={backfill =>
          action.mutate({
            kind: 'settings',
            settings: {
              intervalMinutes: status.intervalMinutes,
              watch: status.watch,
              backfill,
            },
          })
        }
        connected={status.providers.some(p => p.connected)}
        idPrefix="settings-email"
      />
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
