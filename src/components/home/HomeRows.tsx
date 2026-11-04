import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  CircleCheck,
  CircleDot,
  CircleSlash,
  CircleX,
  Clock,
  Star,
} from 'lucide-react'
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
import { Button } from '@/components/ui/button'
import { useRetryRequest } from '@/hooks/useRequests'
import { useWorkAction } from '@/hooks/useWork'
import { formatDuration, formatRelativeTime } from '@/lib/format-time'
import { useViewOpener } from '@/hooks/useViewOpener'
import { runWorkflow } from '@/lib/workflow-actions'
import { cn } from '@/lib/utils'
import type { ActivityStatus } from './activity-model'
import {
  isStale,
  type HomeRetry,
  type HomeRow,
  type HomeSource,
} from './home-model'

const STATUS_ICONS: Record<ActivityStatus, React.ReactNode> = {
  running: <CircleDot className="text-info" />,
  waiting: <Clock className="text-warning" />,
  done: <CircleCheck className="text-success" />,
  failed: <CircleX className="text-destructive" />,
  cancelled: <CircleSlash className="text-muted-foreground" />,
}

export function SourceChip({ source }: { source: HomeSource }) {
  const { t } = useTranslation()
  const label =
    source.kind === 'to' || source.kind === 'from'
      ? t(`home.source.${source.kind}`, { name: source.name })
      : t(`home.source.${source.kind}`)
  return (
    <span className="shrink-0 rounded-full border px-2 py-0.5 text-xs text-muted-foreground">
      {label}
    </span>
  )
}

/** One running Request: title, where it came from, step n of m, elapsed */
export function RunningRow({ row, now }: { row: HomeRow; now: number }) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const stale = isStale(row, now)
  return (
    <button
      type="button"
      onClick={() => open(row.view)}
      className="flex w-full flex-col gap-1 rounded-md px-3 py-2 text-start hover:bg-accent/40"
    >
      <span className="flex items-center gap-2">
        <span aria-hidden className="[&_svg]:size-4">
          {STATUS_ICONS[row.status]}
        </span>
        <span className="sr-only">{t(`home.status.${row.status}`)}</span>
        <span className="min-w-0 flex-1 truncate text-sm font-medium">
          {row.title}
        </span>
        <SourceChip source={row.source} />
        {row.progress && (
          <span className="shrink-0 text-xs text-muted-foreground tabular-nums">
            {t('home.running.step', {
              done: Math.min(row.progress.done + 1, row.progress.total),
              total: row.progress.total,
            })}
          </span>
        )}
      </span>
      <span className="flex items-center gap-2 ps-6 text-xs text-muted-foreground">
        <span className="min-w-0 flex-1 truncate">
          {stale
            ? t('home.running.stale', {
                duration: formatDuration(now - row.updatedAt),
              })
            : row.status === 'waiting'
              ? t('home.running.waiting')
              : row.agent
                ? t('home.running.on', { agent: row.agent })
                : t('home.running.working')}
        </span>
        <span className={cn('shrink-0 tabular-nums', stale && 'text-warning')}>
          {formatDuration(Math.max(0, now - row.startedAt))}
        </span>
      </span>
    </button>
  )
}

/** One finished Request: state, title, where from, when; rate or retry */
export function RecentRow({ row }: { row: HomeRow }) {
  const { t, i18n } = useTranslation()
  const { open } = useViewOpener()
  return (
    <div className="flex items-center gap-2 rounded-md px-3 py-1.5 hover:bg-accent/40">
      <button
        type="button"
        onClick={() => open(row.view)}
        className="flex min-w-0 flex-1 items-center gap-2 text-start"
      >
        <span aria-hidden className="[&_svg]:size-4">
          {STATUS_ICONS[row.status]}
        </span>
        <span className="sr-only">{t(`home.status.${row.status}`)}</span>
        <span className="min-w-0 flex-1 truncate text-sm">{row.title}</span>
        <SourceChip source={row.source} />
        <time
          dateTime={new Date(row.endedAt ?? row.startedAt).toISOString()}
          className="w-16 shrink-0 text-end text-xs text-muted-foreground"
        >
          {formatRelativeTime(row.endedAt ?? row.startedAt, i18n.language)}
        </time>
      </button>
      {row.rating && (
        <Rating workId={row.rating.workId} value={row.rating.value} />
      )}
      {row.retry && <RetryButton retry={row.retry} title={row.title} />}
    </div>
  )
}

/** ★ once, for a delegation that finished; then "Rated n/5" */
function Rating({ workId, value }: { workId: string; value: number | null }) {
  const { t } = useTranslation()
  const action = useWorkAction()
  if (value !== null) {
    return (
      <span className="shrink-0 text-xs text-muted-foreground">
        {t('work.rated', { rating: value })}
      </span>
    )
  }
  return (
    <span
      role="group"
      aria-label={t('work.rate')}
      className="flex shrink-0 items-center"
    >
      {[1, 2, 3, 4, 5].map(stars => (
        <button
          key={stars}
          type="button"
          aria-label={t('work.rateStars', { rating: stars })}
          className="p-0.5 text-muted-foreground hover:text-warning"
          onClick={() =>
            action.mutate(
              { id: workId, action: 'rate', rating: stars },
              {
                onError: e =>
                  toast.error(e instanceof Error ? e.message : String(e)),
              }
            )
          }
        >
          <Star className="size-3.5" />
        </button>
      ))}
    </span>
  )
}

/**
 * Retry: a delegation tries again (the provider still gates it, so no
 * confirmation); a workflow run starts over, which can repeat what it
 * did, so it asks first.
 */
function RetryButton({ retry, title }: { retry: HomeRetry; title: string }) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const retryRequest = useRetryRequest()
  const [confirming, setConfirming] = useState(false)

  const run = async () => {
    if (retry.kind === 'request') {
      try {
        const request = await retryRequest.mutateAsync(retry.requestId)
        open({ kind: 'request', requestId: request.id })
      } catch (e) {
        toast.error(e instanceof Error ? e.message : String(e))
      }
      return
    }
    if (await runWorkflow(retry.workflowId)) toast.success(t('home.retried'))
  }

  return (
    <>
      <Button
        size="sm"
        variant="outline"
        className="h-7 shrink-0"
        disabled={retryRequest.isPending}
        onClick={() =>
          retry.kind === 'workflow' ? setConfirming(true) : void run()
        }
      >
        {t('home.retry')}
      </Button>
      <AlertDialog open={confirming} onOpenChange={setConfirming}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('home.retryConfirm.title', { title })}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('home.retryConfirm.description')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>
              {t('home.retryConfirm.cancel')}
            </AlertDialogCancel>
            <AlertDialogAction onClick={() => void run()}>
              {t('home.retryConfirm.confirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}
