import { useTranslation } from 'react-i18next'
import { CalendarDays, FileText, RefreshCw } from 'lucide-react'
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
import { EmailSignInStatus } from '@/components/email/EmailSignInStatus'
import {
  useWorkspaceAction,
  useWorkspaceSignIn,
  useWorkspaceStatus,
} from '@/hooks/useConnectorHub'
import { formatRelativeTime } from '@/lib/format-time'
import type { WorkspaceAccountStatus, WorkspaceStatus } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { intervalLabel, type CardTone } from '../email/email-model'
import {
  calendarIntervalOptions,
  workspaceCard,
} from '../email/workspace-model'
import { SettingsGroup, SettingsSectionFrame } from '../SettingsFrame'

const TONE: Record<CardTone, string> = {
  muted: 'text-muted-foreground',
  ok: 'text-success',
  warning: 'text-warning',
  error: 'text-destructive',
}

/**
 * Settings › Calendar & files: Outlook calendar, Google Calendar, OneDrive
 * and Google Drive, read-only, each signing in for its own access only;
 * then how often the Chief of Staff checks the calendars.
 */
export function WorkspaceSection() {
  const { t } = useTranslation()
  const status = useWorkspaceStatus()

  return (
    <SettingsSectionFrame
      title={t('settings.section.workspace')}
      description={t('workspace.description')}
    >
      <ul className="list-disc space-y-1 ps-5 text-sm text-muted-foreground">
        <li>{t('workspace.readOnly')}</li>
        <li>{t('workspace.ownAccess')}</li>
      </ul>
      {status.isPending && <Spinner />}
      {status.error && (
        <p className="text-sm text-destructive">{String(status.error)}</p>
      )}
      {status.data && <WorkspaceBody status={status.data} />}
    </SettingsSectionFrame>
  )
}

function WorkspaceBody({ status }: { status: WorkspaceStatus }) {
  const { t, i18n } = useTranslation()
  const calendars = status.accounts.filter(a => a.family === 'calendar')
  const files = status.accounts.filter(a => a.family === 'files')
  const anyCalendar = calendars.some(a => a.connected)
  return (
    <>
      <SettingsGroup title={t('workspace.calendars')}>
        {calendars.map(account => (
          <AccountCard
            key={account.kind}
            status={account}
            checking={status.checking}
          />
        ))}
        {anyCalendar &&
          status.nextCalendarCheckAt !== null &&
          !status.checking && (
            <p className="text-xs text-muted-foreground">
              {t('workspace.nextCheck', {
                when: formatRelativeTime(
                  status.nextCalendarCheckAt,
                  i18n.language
                ),
              })}
            </p>
          )}
      </SettingsGroup>
      <SettingsGroup title={t('workspace.files')}>
        {files.map(account => (
          <AccountCard
            key={account.kind}
            status={account}
            checking={status.checking}
          />
        ))}
      </SettingsGroup>
      <CalendarSchedule status={status} />
    </>
  )
}

function AccountCard({
  status,
  checking,
}: {
  status: WorkspaceAccountStatus
  checking: boolean
}) {
  const { t, i18n } = useTranslation()
  const card = workspaceCard(status)
  const action = useWorkspaceAction()
  const signIn = useWorkspaceSignIn(status.kind)
  const waiting = signIn.state.phase === 'waiting'
  const Icon = status.family === 'calendar' ? CalendarDays : FileText

  return (
    <div className="space-y-3 rounded-md border p-4" data-account={status.id}>
      <div className="flex flex-wrap items-center gap-2">
        <Icon className="size-4 text-muted-foreground" />
        <span className="font-medium">{card.name}</span>
        <span className={cn('text-sm', TONE[card.tone])}>
          {t(`workspace.state.${card.statusKey}`)}
        </span>
      </div>
      {card.account && (
        <p className="text-sm">
          {t('email.connectedAs', { account: card.account })}
        </p>
      )}
      {status.connected && status.family === 'calendar' && (
        <p className="text-xs text-muted-foreground">
          {status.lastCheckedAt !== null
            ? t('workspace.lastChecked', {
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
                  {t('workspace.disconnectBody')}
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>{t('email.cancel')}</AlertDialogCancel>
                <AlertDialogAction
                  onClick={() =>
                    action.mutate({ kind: 'disconnect', account: status.kind })
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

function CalendarSchedule({ status }: { status: WorkspaceStatus }) {
  const { t } = useTranslation()
  const action = useWorkspaceAction()
  const options = calendarIntervalOptions(status.calendarIntervalMinutes)
  return (
    <SettingsGroup title={t('workspace.schedule')}>
      <label className="flex flex-wrap items-center gap-2 text-sm">
        {t('workspace.every')}
        <NativeSelect
          value={String(status.calendarIntervalMinutes)}
          onChange={event =>
            action.mutate({
              kind: 'interval',
              minutes: Number(event.target.value),
            })
          }
          aria-label={t('workspace.every')}
        >
          {options.map(minutes => (
            <NativeSelectOption key={minutes} value={String(minutes)}>
              {intervalLabel(t, minutes)}
            </NativeSelectOption>
          ))}
        </NativeSelect>
      </label>
      <p className="text-xs text-muted-foreground">{t('workspace.oneRun')}</p>
      {action.error && (
        <p className="text-sm text-destructive">{String(action.error)}</p>
      )}
    </SettingsGroup>
  )
}
