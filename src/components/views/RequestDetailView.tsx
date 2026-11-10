import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { ArrowLeft, Copy, Eye, RotateCcw, Star, Workflow } from 'lucide-react'
import { toast } from 'sonner'
import { MarkdownContent } from '@/components/agent-console/MarkdownContent'
import { AskedBlock } from '@/components/requests/AskedBlock'
import { RequestBanner } from '@/components/requests/RequestBanner'
import { RequestSteps } from '@/components/requests/RequestSteps'
import { SaveAsWorkflowDialog } from '@/components/requests/SaveAsWorkflowDialog'
import {
  canStop,
  focusStep,
  localStepPosition,
} from '@/components/requests/request-model'
import { useNow } from '@/components/agents/task-utils'
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
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
} from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { errorMessage } from '@/hooks/useDelegation'
import { errorKind } from '@/lib/friendly-error'
import { plainError } from '@/lib/toast-error'
import {
  requestQueryKeys,
  useCompleteRequestStep,
  useRequest,
  useRerunRequest,
} from '@/hooks/useRequests'
import { client } from '@/lib/client'
import { watchInCompanion } from '@/lib/companion-actions'
import type { JsonValue, Request, RequestStatus } from '@/lib/bindings'
import { formatDuration } from '@/lib/format-time'
import { queryClient } from '@/lib/query-client'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'

const STATUS_TONES: Record<RequestStatus, string> = {
  planning: 'text-muted-foreground',
  running: 'text-info',
  waiting: 'text-warning',
  done: 'text-success',
  failed: 'text-destructive',
  cancelled: 'text-muted-foreground',
}

type Tab = 'steps' | 'outputs' | 'activity' | 'details'

/**
 * One Request: where it is, why it waits, and the button that moves it on
 * (request-detail.md). Phase 1 covers delegated and gateway tasks.
 */
export function RequestDetailView({
  requestId,
  stepId,
}: {
  requestId: string
  stepId?: string
}) {
  const { t } = useTranslation()
  const request = useRequest(requestId)

  if (request.isPending) return <RequestSkeleton />
  if (request.isError) {
    const message = errorMessage(request.error)
    const missing = message.includes('not found')
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyTitle>
            {t(missing ? 'request.notFound.title' : 'request.loadError.title')}
          </EmptyTitle>
          <EmptyDescription>
            {missing ? t('request.notFound.description') : message}
          </EmptyDescription>
        </EmptyHeader>
        <EmptyContent>
          {missing ? (
            <Button
              onClick={() =>
                useNavigationStore.getState().navigate({ kind: 'home' })
              }
            >
              {t('request.backHome')}
            </Button>
          ) : (
            <Button onClick={() => void request.refetch()}>
              {t('request.tryAgain')}
            </Button>
          )}
        </EmptyContent>
      </Empty>
    )
  }
  return <RequestPage request={request.data} stepId={stepId} />
}

function RequestPage({
  request,
  stepId,
}: {
  request: Request
  stepId?: string
}) {
  const { t, i18n } = useTranslation()
  const active = request.endedAt === null
  const now = useNow(active)
  const [tab, setTab] = useState<Tab>('steps')
  const [stopping, setStopping] = useState(false)
  const [savingWorkflow, setSavingWorkflow] = useState(false)
  const local = request.direction === 'local' && request.capability === ''
  const completeStep = useCompleteRequestStep(request.id)
  const rerun = useRerunRequest()
  const elapsed = (request.endedAt ?? now) - request.startedAt
  const name = request.counterpart ?? t('request.someone')
  const ended =
    request.status === 'done' ||
    request.status === 'failed' ||
    request.status === 'cancelled'

  const stop = async () => {
    const taskId = request.stuck?.taskId ?? request.taskIds.at(-1) ?? request.id
    const result = local
      ? await client.cancelRequest(request.id)
      : await client.cancelTask(taskId, null)
    if (result.status === 'error') {
      toast.error(result.error)
      return
    }
    toast.success(t('request.done.stopped'))
    void queryClient.invalidateQueries({ queryKey: requestQueryKeys.all })
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 flex-col gap-1 border-b px-4 py-3">
        <Button
          variant="ghost"
          size="sm"
          className="-ms-2 self-start"
          onClick={() => useNavigationStore.getState().goBack()}
        >
          <ArrowLeft className="rtl:rotate-180" />
          {t('request.back')}
        </Button>
        <div className="flex flex-wrap items-center gap-2">
          <h1 className="min-w-0 flex-1 truncate text-lg font-semibold">
            {request.title}
          </h1>
          {!ended && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => void watchInCompanion(request.id)}
            >
              <Eye />
              {t('request.watchInCompanion')}
            </Button>
          )}
          {local && (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => setSavingWorkflow(true)}
            >
              <Workflow />
              {t('request.saveWorkflow.button')}
            </Button>
          )}
          {local && ended && (
            <Button
              size="sm"
              variant="outline"
              disabled={rerun.isPending}
              onClick={() =>
                rerun.mutate(request.id, {
                  onSuccess: id =>
                    useNavigationStore
                      .getState()
                      .navigate({ kind: 'request', requestId: id }),
                  onError: error => toast.error(errorMessage(error)),
                })
              }
            >
              <RotateCcw />
              {t('request.doAgain')}
            </Button>
          )}
          {canStop(request) && (
            <Button
              size="sm"
              variant="outline"
              onClick={() => setStopping(true)}
            >
              {t('request.stop')}
            </Button>
          )}
        </div>
        <p className="flex flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
          <span>{t(`request.source.${request.direction}`, { name })}</span>
          <span aria-hidden>·</span>
          <span>
            {t('request.started', {
              time: new Date(request.startedAt).toLocaleTimeString(
                i18n.language,
                {
                  hour: '2-digit',
                  minute: '2-digit',
                }
              ),
            })}
          </span>
          <span aria-hidden>·</span>
          <span className="tabular-nums" data-testid="request-elapsed">
            {formatDuration(Math.max(0, elapsed))}
          </span>
          <span aria-hidden>·</span>
          <span className={cn('font-medium', STATUS_TONES[request.status])}>
            {t(`request.status.${request.status}`)}
          </span>
          {request.planSteps > 0 && !ended && (
            <>
              <span aria-hidden>·</span>
              <span>
                {t('request.stepOf', {
                  current: request.currentStep,
                  total: request.planSteps,
                })}
              </span>
            </>
          )}
        </p>
      </header>

      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto flex w-full max-w-4xl flex-col gap-4 p-4">
          <AskedBlock request={request} />
          {request.stuck && (
            <RequestBanner
              request={request}
              blocker={request.stuck}
              now={now}
              onShowDetails={() => setTab('details')}
            />
          )}
          {ended && request.status === 'done' && (
            <ResultBlock request={request} />
          )}

          <Tabs value={tab} onValueChange={value => setTab(value as Tab)}>
            <TabsList>
              <TabsTrigger value="steps">{t('request.tabs.steps')}</TabsTrigger>
              <TabsTrigger value="outputs">
                {t('request.tabs.outputs')}
              </TabsTrigger>
              <TabsTrigger value="activity">
                {t('request.tabs.activity')}
              </TabsTrigger>
              <TabsTrigger value="details">
                {t('request.tabs.details')}
              </TabsTrigger>
            </TabsList>
            <TabsContent value="steps" className="pt-2">
              <RequestSteps
                request={request}
                now={now}
                focused={focusStep(request, stepId)}
                onComplete={
                  local
                    ? step => {
                        const position = localStepPosition(step)
                        if (position === null) return
                        completeStep.mutate(position, {
                          onError: error => toast.error(errorMessage(error)),
                        })
                      }
                    : undefined
                }
              />
            </TabsContent>
            <TabsContent value="outputs" className="pt-2">
              <Outputs request={request} />
            </TabsContent>
            <TabsContent value="activity" className="pt-2">
              <Activity request={request} />
            </TabsContent>
            <TabsContent value="details" className="pt-2">
              <Details request={request} />
            </TabsContent>
          </Tabs>
        </div>
      </ScrollArea>

      {local && (
        <SaveAsWorkflowDialog
          requestId={request.id}
          open={savingWorkflow}
          onOpenChange={setSavingWorkflow}
        />
      )}

      <AlertDialog open={stopping} onOpenChange={setStopping}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('request.confirm.stop.title')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {request.direction === 'local'
                ? t('request.confirm.stop.local')
                : t('request.confirm.stop.description', { member: name })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('request.confirm.keep')}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void stop()}>
              {t('request.stop')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function Result({ value }: { value: JsonValue }) {
  return typeof value === 'string' ? (
    <MarkdownContent content={value} />
  ) : (
    <pre dir="ltr" className="overflow-auto rounded bg-muted p-2 text-xs">
      {JSON.stringify(value, null, 2)}
    </pre>
  )
}

/** Done: the result first, and (outgoing) how the provider did */
function ResultBlock({ request }: { request: Request }) {
  const { t } = useTranslation()
  const [rating, setRating] = useState(request.rating)

  const rate = async (stars: number) => {
    const result = await client.rateWork(request.id, stars)
    if (result.status === 'error') {
      toast.error(result.error)
      return
    }
    setRating(stars)
  }

  return (
    <section
      aria-label={t('request.result')}
      className="flex flex-col gap-2 rounded-lg border p-4"
    >
      <h2 className="text-sm font-semibold">{t('request.result')}</h2>
      {request.result !== null ? (
        <div className="max-h-72 overflow-y-auto text-sm">
          <Result value={request.result} />
        </div>
      ) : (
        <p className="text-sm text-muted-foreground">{t('request.noResult')}</p>
      )}
      {request.direction === 'outgoing' &&
        (rating ? (
          <p className="text-xs text-muted-foreground">
            {t('request.rated', { count: rating })}
          </p>
        ) : (
          <div className="flex items-center gap-2 text-xs">
            <span>
              {t('request.rate', {
                member: request.counterpart ?? t('request.someone'),
              })}
            </span>
            {[1, 2, 3, 4, 5].map(stars => (
              <button
                key={stars}
                type="button"
                aria-label={t('request.rateStars', { count: stars })}
                onClick={() => void rate(stars)}
                className="text-muted-foreground hover:text-warning"
              >
                <Star aria-hidden className="size-4" />
              </button>
            ))}
          </div>
        ))}
    </section>
  )
}

function Outputs({ request }: { request: Request }) {
  const { t } = useTranslation()
  if (request.result === null && request.error === null) {
    return (
      <p className="text-sm text-muted-foreground">
        {t('request.outputs.empty')}
      </p>
    )
  }
  return (
    <div className="flex flex-col gap-3 text-sm">
      {request.result !== null && <Result value={request.result} />}
      {request.error &&
        (errorKind(request.error) === 'dailyLimit' ? (
          <p
            className="text-muted-foreground"
            data-testid="request-daily-limit"
          >
            {plainError(request.error)}
          </p>
        ) : (
          <p role="alert" className="text-destructive" dir="auto">
            {request.error}
          </p>
        ))}
    </div>
  )
}

type ActivityFilter = 'all' | 'state' | 'messages'

function Activity({ request }: { request: Request }) {
  const { t, i18n } = useTranslation()
  const [filter, setFilter] = useState<ActivityFilter>('all')
  const rows = request.activity.filter(
    row => filter === 'all' || (filter === 'state') === (row.kind === 'state')
  )
  return (
    <div className="flex flex-col gap-2">
      <div className="flex gap-1">
        {(['all', 'state', 'messages'] as const).map(value => (
          <Button
            key={value}
            size="sm"
            variant={filter === value ? 'secondary' : 'ghost'}
            className="h-7"
            onClick={() => setFilter(value)}
          >
            {t(`request.activity.${value}`)}
          </Button>
        ))}
      </div>
      <ol
        aria-label={t('request.tabs.activity')}
        className="flex flex-col divide-y text-xs"
      >
        {rows.map((row, index) => (
          <li
            key={`${row.at}-${index}`}
            className="flex flex-col gap-0.5 py-1.5"
          >
            <div className="flex items-center gap-2">
              <time className="tabular-nums text-muted-foreground">
                {new Date(row.at).toLocaleTimeString(i18n.language, {
                  hour: '2-digit',
                  minute: '2-digit',
                  second: '2-digit',
                })}
              </time>
              <span className="font-medium">
                {row.kind === 'state'
                  ? `${row.fromState ?? '—'} → ${row.toState ?? ''}`
                  : row.kind}
              </span>
              {row.reason && (
                <span dir="auto" className="truncate text-muted-foreground">
                  {row.reason}
                </span>
              )}
            </div>
            {row.payload !== null && (
              <details>
                <summary className="cursor-pointer text-muted-foreground">
                  {t('request.activity.payload')}
                </summary>
                <pre dir="ltr" className="overflow-auto rounded bg-muted p-2">
                  {JSON.stringify(row.payload, null, 2)}
                </pre>
              </details>
            )}
          </li>
        ))}
      </ol>
    </div>
  )
}

function CopyValue({ value }: { value: string }) {
  const { t } = useTranslation()
  return (
    <span className="flex items-center gap-1">
      <code dir="ltr" className="font-mono">
        {value}
      </code>
      <Button
        size="icon"
        variant="ghost"
        className="size-6"
        aria-label={t('request.copy')}
        onClick={() =>
          void navigator.clipboard
            .writeText(value)
            .then(() => toast.success(t('request.copiedValue')))
        }
      >
        <Copy className="size-3" />
      </Button>
    </span>
  )
}

function Details({ request }: { request: Request }) {
  const { t } = useTranslation()
  const routed = request.steps.find(s => s.stage === 'routed')?.detail
  const rows: [string, React.ReactNode][] = [
    [t('request.details.capability'), request.capability],
    [
      t('request.details.direction'),
      t(`request.direction.${request.direction}`),
    ],
    ...(routed
      ? ([[t('request.details.decision'), routed]] as [
          string,
          React.ReactNode,
        ][])
      : []),
    [
      t('request.details.requester'),
      <CopyValue key="r" value={request.requesterInstance} />,
    ],
    [
      t('request.details.provider'),
      <CopyValue key="p" value={request.providerInstance} />,
    ],
    [
      t('request.details.tasks'),
      <span key="t" className="flex flex-col">
        {request.taskIds.map(id => (
          <CopyValue key={id} value={id} />
        ))}
      </span>,
    ],
    [
      t('request.details.eta'),
      request.etaSeconds !== null
        ? formatDuration(request.etaSeconds * 1000)
        : '—',
    ],
    [
      t('request.details.usage'),
      request.costUsd !== null || request.inputTokens > 0
        ? t('request.details.usageValue', {
            input: request.inputTokens,
            output: request.outputTokens,
            cost:
              request.costUsd !== null ? `$${request.costUsd.toFixed(3)}` : '—',
          })
        : '—',
    ],
  ]
  return (
    <dl className="grid grid-cols-[minmax(8rem,auto)_1fr] gap-x-4 gap-y-2 text-sm">
      {rows.map(([label, value]) => (
        <div key={label} className="contents">
          <dt className="text-muted-foreground">{label}</dt>
          <dd className="min-w-0 break-words">{value}</dd>
        </div>
      ))}
      {request.stuck?.reason && (
        <div className="contents">
          <dt className="text-muted-foreground">{t('request.rawReason')}</dt>
          <dd dir="auto" className="whitespace-pre-wrap text-xs">
            {request.stuck.reason}
          </dd>
        </div>
      )}
    </dl>
  )
}

function RequestSkeleton() {
  return (
    <div className="flex flex-col gap-3 p-4" aria-busy>
      <Skeleton className="h-7 w-1/2" />
      <Skeleton className="h-4 w-1/3" />
      {[0, 1, 2, 3].map(i => (
        <Skeleton key={i} className="h-8" />
      ))}
    </div>
  )
}
