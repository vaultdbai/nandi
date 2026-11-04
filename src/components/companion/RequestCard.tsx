import { useTranslation } from 'react-i18next'
import { ChevronRight, MessageSquare } from 'lucide-react'
import { bannerSentenceKey } from '@/components/requests/request-model'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { useRequest } from '@/hooks/useRequests'
import { cn } from '@/lib/utils'
import { useViewOpener } from '@/hooks/useViewOpener'

const STATUS_TONE: Record<string, string> = {
  running: 'text-info',
  waiting: 'text-warning',
  done: 'text-success',
  failed: 'text-destructive',
  cancelled: 'text-muted-foreground',
  planning: 'text-muted-foreground',
}

/**
 * One Request as a small live card: title, status, step n of m, and why
 * it's stuck. Used where chat cites a Request (`nandi://request/<id>`) and
 * for the Request pinned with Watch in companion. Spans only, so it can sit
 * inside a Markdown paragraph.
 */
export function RequestCard({
  requestId,
  onAsk,
}: {
  requestId: string
  /** Pins the conversation to this Request */
  onAsk?: (requestId: string) => void
}) {
  const { t } = useTranslation()
  const { open } = useViewOpener()
  const { data: request, isPending, isError } = useRequest(requestId)

  if (isPending) {
    return (
      <span className="my-1 block">
        <Skeleton className="h-12 w-full" />
      </span>
    )
  }
  if (isError || !request) {
    return (
      <span className="my-1 block rounded-md border px-3 py-2 text-xs text-muted-foreground">
        {t('companion.requestGone')}
      </span>
    )
  }

  const stuck = request.stuck
  const names = {
    member: stuck?.member ?? request.counterpart ?? t('request.someone'),
    agent: stuck?.agentName ?? t('request.yourAgent'),
  }

  return (
    <span
      className="my-1 flex items-start gap-2 rounded-md border bg-card px-3 py-2 text-start"
      data-testid="request-card"
    >
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">
          {request.title}
        </span>
        <span className="flex flex-wrap items-center gap-x-2 text-xs">
          <span className={cn(STATUS_TONE[request.status])}>
            {t(`request.status.${request.status}`)}
          </span>
          {request.planSteps > 0 && (
            <span className="text-muted-foreground tabular-nums">
              {t('request.stepOf', {
                current: Math.min(request.currentStep, request.planSteps),
                total: request.planSteps,
              })}
            </span>
          )}
        </span>
        {stuck && (
          <span className="block truncate text-xs text-muted-foreground">
            {t(bannerSentenceKey(request, stuck), names)}
          </span>
        )}
      </span>
      {onAsk && (
        <Button
          size="icon-sm"
          variant="ghost"
          className="size-7 shrink-0"
          aria-label={t('companion.askAbout', { title: request.title })}
          title={t('companion.askAbout', { title: request.title })}
          onClick={() => onAsk(request.id)}
        >
          <MessageSquare />
        </Button>
      )}
      <Button
        size="icon-sm"
        variant="ghost"
        className="size-7 shrink-0"
        aria-label={t('companion.openRequest', { title: request.title })}
        title={t('companion.openRequest', { title: request.title })}
        onClick={() => open({ kind: 'request', requestId: request.id })}
      >
        <ChevronRight className="rtl:rotate-180" />
      </Button>
    </span>
  )
}
