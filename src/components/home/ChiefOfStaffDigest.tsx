import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Sparkles } from 'lucide-react'
import { toast } from 'sonner'
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
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
  useApproveQueuedWork,
  useChiefOfStaffDigest,
  useDismissQueuedWork,
  useMarkDigestSeen,
} from '@/hooks/useChiefOfStaffDigest'
import { useViewOpener } from '@/hooks/useViewOpener'
import { cn } from '@/lib/utils'
import {
  digestRows,
  digestSummary,
  digestTitleKey,
  queueRows,
  type DigestRow,
  type QueueRow,
} from './digest-model'

/**
 * Home's "Since you were away": what the Chief of Staff started, updated
 * or finished on its own, and what waits on the user. "Got it" marks it
 * seen; needs-you rows stay until they are done.
 */
export function ChiefOfStaffDigest() {
  const { t, i18n } = useTranslation()
  const { open } = useViewOpener()
  const digest = useChiefOfStaffDigest()
  const markSeen = useMarkDigestSeen()
  const approve = useApproveQueuedWork()
  const dismiss = useDismissQueuedWork()
  const [confirming, setConfirming] = useState<QueueRow | null>(null)
  if (!digest.data) return null
  if (digest.data.items.length === 0 && digest.data.queue.length === 0) {
    return null
  }

  const rows = digestRows(digest.data, i18n.language)
  const queue = queueRows(digest.data, i18n.language)
  const summary = digestSummary(digest.data)
    .map(part =>
      t(`home.chiefOfStaffDigest.summary.${part.kind}`, { count: part.count })
    )
    .join(' · ')

  const gotIt = async () => {
    try {
      await markSeen.mutateAsync()
    } catch (e) {
      toast.error(e instanceof Error ? e.message : String(e))
    }
  }

  const approveRow = async (row: QueueRow) => {
    setConfirming(null)
    try {
      const requestId = await approve.mutateAsync(row.id)
      const message =
        row.action === 'update' && row.requestTitle
          ? t('home.chiefOfStaffDigest.queue.toast.addedTo', {
              title: row.requestTitle,
            })
          : t('home.chiefOfStaffDigest.queue.toast.started')
      toast.success(message, {
        action: {
          label: t('home.chiefOfStaffDigest.queue.toast.open'),
          onClick: () => open({ kind: 'request', requestId }),
        },
      })
    } catch (e) {
      toast.error(t('home.chiefOfStaffDigest.queue.toast.approveFailed'), {
        description: e instanceof Error ? e.message : String(e),
      })
    }
  }

  const dismissRow = async (row: QueueRow) => {
    try {
      await dismiss.mutateAsync(row.id)
    } catch (e) {
      toast.error(t('home.chiefOfStaffDigest.queue.toast.dismissFailed'), {
        description: e instanceof Error ? e.message : String(e),
      })
    }
  }

  return (
    <Card data-testid="home-cos-digest">
      <CardHeader className="flex flex-row items-start gap-3">
        <Sparkles
          aria-hidden
          className="mt-1 size-4 shrink-0 text-muted-foreground"
        />
        <div className="min-w-0 flex-1">
          <CardTitle className="text-base">
            {t(digestTitleKey(digest.data))}
          </CardTitle>
          <p className="text-sm text-muted-foreground">{summary}</p>
        </div>
        {rows.length > 0 && (
          <Button
            size="sm"
            variant="outline"
            disabled={markSeen.isPending}
            onClick={() => void gotIt()}
          >
            {t('home.chiefOfStaffDigest.gotIt')}
          </Button>
        )}
      </CardHeader>
      <CardContent className="flex flex-col gap-1">
        {queue.length > 0 && (
          <section
            data-testid="home-cos-queue"
            aria-label={t('home.chiefOfStaffDigest.queue.title')}
            className="mb-2 flex flex-col gap-2"
          >
            <h3 className="px-2 text-sm font-medium">
              {t('home.chiefOfStaffDigest.queue.title')}
            </h3>
            {queue.map(row => (
              <QueueRowView
                key={row.id}
                row={row}
                busy={approve.isPending || dismiss.isPending}
                onApprove={() =>
                  row.needsConfirm ? setConfirming(row) : void approveRow(row)
                }
                onDismiss={() => void dismissRow(row)}
              />
            ))}
          </section>
        )}
        {rows.map((row, index) => (
          <DigestRowButton
            // One Request can show twice (waiting on you, and updated)
            key={`${row.kind}-${row.requestId}-${index}`}
            row={row}
            onOpen={() => open({ kind: 'request', requestId: row.requestId })}
          />
        ))}
      </CardContent>
      <AlertDialog
        open={confirming !== null}
        onOpenChange={next => {
          if (!next) setConfirming(null)
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
            <AlertDialogAction
              onClick={() => confirming && void approveRow(confirming)}
            >
              {t('home.chiefOfStaffDigest.queue.confirm.action')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  )
}

function QueueRowView({
  row,
  busy,
  onApprove,
  onDismiss,
}: {
  row: QueueRow
  busy: boolean
  onApprove: () => void
  onDismiss: () => void
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
      <p className="line-clamp-2 font-medium">{row.text}</p>
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
      <div className="flex items-center justify-end gap-2">
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

function DigestRowButton({
  row,
  onOpen,
}: {
  row: DigestRow
  onOpen: () => void
}) {
  const { t } = useTranslation()
  return (
    <button
      type="button"
      onClick={onOpen}
      className="flex min-w-0 flex-col gap-0.5 rounded-md px-2 py-2 text-start text-sm hover:bg-muted"
    >
      <span className="flex min-w-0 items-center gap-2">
        <Badge
          variant={row.kind === 'needsYou' ? 'default' : 'secondary'}
          className="shrink-0"
        >
          {t(row.labelKey)}
        </Badge>
        <span className="min-w-0 flex-1 truncate font-medium">{row.title}</span>
        <span className="shrink-0 text-xs text-muted-foreground">
          {t(row.sourceKey)} · {row.when}
        </span>
      </span>
      {row.text && (
        <span className={cn('truncate ps-2 text-xs text-muted-foreground')}>
          {row.text}
        </span>
      )}
    </button>
  )
}
