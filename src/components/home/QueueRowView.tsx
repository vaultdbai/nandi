import { useTranslation } from 'react-i18next'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type { QueueRow } from './digest-model'

/**
 * One screened item waiting for the user's OK: what it would do, who sent
 * it, the screening verdict and the plan. `onOpen` (Home) opens its thread.
 */
export function QueueRowView({
  row,
  busy,
  onApprove,
  onDismiss,
  onOpen,
  onAlwaysAllow,
  clamp = true,
}: {
  row: QueueRow
  busy: boolean
  onApprove: () => void
  onDismiss: () => void
  onOpen?: () => void
  /** "Always allow mail from <domain>": shown when the row has a domain */
  onAlwaysAllow?: () => void
  /** Cut the text to two lines (Home); the thread page shows it whole */
  clamp?: boolean
}) {
  const { t } = useTranslation()
  const label =
    row.action !== 'start' && row.requestTitle
      ? t(
          row.action === 'run'
            ? 'home.chiefOfStaffDigest.queue.run'
            : 'home.chiefOfStaffDigest.queue.addTo',
          { title: row.requestTitle }
        )
      : t('home.chiefOfStaffDigest.queue.start')
  const origin = [row.sender, `${t(row.sourceKey)} · ${row.when}`]
    .filter(Boolean)
    .join(' · ')
  const text = (
    <p className={cn('font-medium', clamp && 'line-clamp-2')}>{row.text}</p>
  )
  return (
    <div
      data-testid="home-cos-queue-row"
      data-verdict={row.verdict}
      className="flex min-w-0 flex-col gap-1 rounded-md border px-3 py-2 text-sm"
    >
      <span className="flex min-w-0 items-center gap-2">
        <Badge variant="secondary" className="min-w-0 truncate">
          {label}
        </Badge>
        {row.verdict === 'suspicious' && (
          <Badge
            variant="outline"
            className="shrink-0 border-warning/40 bg-warning/10 text-warning"
          >
            {t('home.chiefOfStaffDigest.queue.verdict.suspicious')}
          </Badge>
        )}
        {row.verdict === 'blocked' && (
          <Badge variant="destructive" className="shrink-0">
            {t('home.chiefOfStaffDigest.queue.verdict.blocked')}
          </Badge>
        )}
      </span>
      {onOpen ? (
        <button
          type="button"
          onClick={onOpen}
          className="min-w-0 text-start hover:underline"
        >
          {text}
        </button>
      ) : (
        text
      )}
      <p className="truncate text-xs text-muted-foreground">{origin}</p>
      {row.reasons.length > 0 && (
        <ul
          className={cn(
            'list-disc ps-5 text-xs',
            row.verdict === 'blocked' ? 'text-destructive' : 'text-warning'
          )}
        >
          {row.reasons.map(reason => (
            <li key={reason}>{reason}</li>
          ))}
        </ul>
      )}
      {row.planSteps.length > 0 && (
        <p className="text-xs text-muted-foreground">
          {t('home.chiefOfStaffDigest.queue.plan', {
            steps: row.planSteps.join(' → '),
          })}
        </p>
      )}
      <div className="flex items-center justify-end gap-2">
        {onAlwaysAllow && row.alwaysAllowDomain && (
          <Button
            size="sm"
            variant="ghost"
            className="me-auto"
            disabled={busy}
            onClick={onAlwaysAllow}
          >
            {t('autoApprove.shortcut.action', {
              domain: row.alwaysAllowDomain,
            })}
          </Button>
        )}
        {row.canApprove ? (
          <Button size="sm" disabled={busy} onClick={onApprove}>
            {t('home.chiefOfStaffDigest.queue.approve')}
          </Button>
        ) : (
          <span className="me-auto text-xs text-muted-foreground">
            {t('home.chiefOfStaffDigest.queue.blocked')}
          </span>
        )}
        <Button size="sm" variant="outline" disabled={busy} onClick={onDismiss}>
          {t('home.chiefOfStaffDigest.queue.dismiss')}
        </Button>
      </div>
    </div>
  )
}

/** "Approve a suspicious item?": asked before a suspicious item starts */
export function ApproveSuspiciousDialog({
  open,
  onCancel,
  onApprove,
}: {
  open: boolean
  onCancel: () => void
  onApprove: () => void
}) {
  const { t } = useTranslation()
  return (
    <AlertDialog
      open={open}
      onOpenChange={next => {
        if (!next) onCancel()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t('home.chiefOfStaffDigest.queue.confirm.title')}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t('home.chiefOfStaffDigest.queue.confirm.description')}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>
            {t('home.chiefOfStaffDigest.queue.confirm.cancel')}
          </AlertDialogCancel>
          <AlertDialogAction onClick={onApprove}>
            {t('home.chiefOfStaffDigest.queue.confirm.action')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}

/** "Always allow mail from <domain>?": says what the rule does and does not let through */
export function AlwaysAllowDialog({
  domain,
  onCancel,
  onConfirm,
}: {
  /** The domain being asked about; `null` keeps the dialog closed */
  domain: string | null
  onCancel: () => void
  onConfirm: () => void
}) {
  const { t } = useTranslation()
  return (
    <AlertDialog
      open={domain !== null}
      onOpenChange={next => {
        if (!next) onCancel()
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t('autoApprove.shortcut.title', { domain })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {t('autoApprove.shortcut.description', { domain })}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>
            {t('autoApprove.shortcut.cancel')}
          </AlertDialogCancel>
          <AlertDialogAction onClick={onConfirm}>
            {t('autoApprove.shortcut.confirm')}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
