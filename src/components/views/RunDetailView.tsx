/**
 * Run detail: header (agent · input · time, Stop or Re-run, live elapsed),
 * tabs (Overview, Timeline, Outputs, Files, Logs, Analysis, Approvals), and
 * the "Ask about this run" panel (~65/35 split; the panel collapses behind
 * a toggle on narrow widths). The run refreshes live on its events.
 */
import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  ArrowLeft,
  Bot,
  ChevronDown,
  ChevronRight,
  Copy,
  CornerLeftUp,
  History,
  Layers,
  List,
  RotateCcw,
  Square,
} from 'lucide-react'
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
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { AgentAvatar } from '@/components/agent-console/AgentAvatar'
import { RunAskBox, RunSummaryTab } from '@/components/agent-console/RunSummary'
import {
  RunApprovals,
  RunFiles,
  RunLogs,
  RunMetadata,
  RunOutputs,
} from '@/components/agent-console/RunDetailTabs'
import { MarkdownContent } from '@/components/agent-console/MarkdownContent'
import { RunKeyResults } from '@/components/agent-console/RunKeyResults'
import { RunPlanCard } from '@/components/agent-console/RunPlanCard'
import { RunTimeline } from '@/components/agent-console/RunTimeline'
import { isActive } from '@/components/agent-console/run-detail-model'
import {
  inputHeadline,
  rerunNeedsConfirm,
  stopNeedsConfirm,
} from '@/components/agent-console/run-logs-model'
import { useNow } from '@/components/agents/task-utils'
import { AskedBlock } from '@/components/requests/AskedBlock'
import { useRequest, useRequestForRun } from '@/hooks/useRequests'
import { analysisErrorMessage, useRunAnalysis } from '@/hooks/useRunAnalysis'
import { useRunDetail } from '@/hooks/useSnapshots'
import {
  cancelQueuedRun,
  cancelTurn,
  sendAgentMessage,
} from '@/lib/agent-actions'
import { displayAgent } from '@/lib/agent-identity'
import { formatDuration } from '@/lib/format-time'
import { RUN_STATUS_CLASSES } from '@/lib/run-status'
import type { SourceRef } from '@/lib/bindings'
import type { RunDetail } from '@/lib/tauri-bindings'
import {
  RUN_DETAIL_TABS,
  useNavigationStore,
  type RunDetailTab,
} from '@/store/navigation-store'

/** Run IDs are UUIDs; links show the first block */
function shortId(id: string): string {
  return id.length > 12 ? id.slice(0, 8) : id
}

export function RunDetailView({
  runId,
  tab,
}: {
  runId: string
  /** Opens this tab instead of the default one */
  tab?: RunDetailTab
}) {
  const { t } = useTranslation()
  const { data: detail, isLoading, error, refetch } = useRunDetail(runId)

  if (isLoading) return <RunDetailSkeleton />

  if (!detail) {
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <History />
          </EmptyMedia>
          <EmptyTitle>
            {error ? t('runDetail.loadFailed') : t('runDetail.gone')}
          </EmptyTitle>
          {error && (
            <EmptyDescription>{analysisErrorMessage(error)}</EmptyDescription>
          )}
        </EmptyHeader>
        <EmptyContent>
          {error ? (
            <Button size="sm" onClick={() => void refetch()}>
              {t('runDetail.tryAgain')}
            </Button>
          ) : (
            <Button
              size="sm"
              variant="outline"
              onClick={() => useNavigationStore.getState().goBack()}
            >
              {t('views.back')}
            </Button>
          )}
        </EmptyContent>
      </Empty>
    )
  }

  return <RunDetailLayout detail={detail} initialTab={tab} />
}

function RunDetailSkeleton() {
  return (
    <div className="flex h-full flex-col" aria-busy>
      <div className="flex flex-col gap-2 border-b px-4 py-4">
        <Skeleton className="h-6 w-2/3" />
        <Skeleton className="h-4 w-1/3" />
      </div>
      <div className="flex flex-col gap-3 p-4">
        {[0, 1, 2, 3].map(i => (
          <Skeleton key={i} className="h-10 w-full" />
        ))}
      </div>
    </div>
  )
}

function RunActions({ detail }: { detail: RunDetail }) {
  const { t } = useTranslation()
  const { run } = detail
  const navigate = useNavigationStore(state => state.navigate)

  const copyId = () => {
    void navigator.clipboard.writeText(run.id).then(
      () => toast.success(t('runDetail.actions.copied')),
      () => toast.error(t('runDetail.actions.copyFailed'))
    )
  }

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button size="sm" variant="outline">
          {t('runDetail.actions.label')}
          <ChevronDown />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        <DropdownMenuItem onSelect={copyId}>
          <Copy />
          {t('runDetail.actions.copyId')}
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() => navigate({ kind: 'agent', agentId: run.agentId })}
        >
          <Bot />
          {t('runDetail.actions.openAgent')}
        </DropdownMenuItem>
        <DropdownMenuItem
          onSelect={() =>
            navigate({ kind: 'agent', agentId: run.agentId, tab: 'runs' })
          }
        >
          <List />
          {t('runDetail.actions.viewAllRuns')}
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

/**
 * A button that runs `onConfirm` at once, or first asks in a dialog when
 * `confirm` says what could happen
 */
function ConfirmingButton({
  confirm,
  onConfirm,
  children,
  className,
  variant = 'outline',
}: {
  confirm: { title: string; description: string; action: string } | null
  onConfirm: () => void
  children: React.ReactNode
  className?: string
  variant?: 'outline' | 'destructive'
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <>
      <Button
        size="sm"
        variant={variant}
        className={className}
        onClick={() => (confirm ? setOpen(true) : onConfirm())}
      >
        {children}
      </Button>
      {confirm && (
        <AlertDialog open={open} onOpenChange={setOpen}>
          <AlertDialogContent>
            <AlertDialogHeader>
              <AlertDialogTitle>{confirm.title}</AlertDialogTitle>
              <AlertDialogDescription>
                {confirm.description}
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel>
                {t('runDetail.confirm.cancel')}
              </AlertDialogCancel>
              <AlertDialogAction onClick={onConfirm}>
                {confirm.action}
              </AlertDialogAction>
            </AlertDialogFooter>
          </AlertDialogContent>
        </AlertDialog>
      )}
    </>
  )
}

/** Stop while the run is going or waiting; asks first if it did outward work */
function StopButton({ detail }: { detail: RunDetail }) {
  const { t } = useTranslation()
  const { run } = detail
  const stop = () => {
    const done =
      run.status === 'queued'
        ? cancelQueuedRun(run.id)
        : cancelTurn(run.agentId, run.id)
    void done.then(() => toast.success(t('runDetail.stop.stopped')))
  }
  return (
    <ConfirmingButton
      variant="destructive"
      onConfirm={stop}
      confirm={
        run.status !== 'queued' && stopNeedsConfirm(detail)
          ? {
              title: t('runDetail.stop.confirmTitle'),
              description: t('runDetail.stop.confirmDescription'),
              action: t('runDetail.stop.confirmAction'),
            }
          : null
      }
    >
      <Square />
      {t('runDetail.stop.button')}
    </ConfirmingButton>
  )
}

/**
 * Re-run once the run has ended: the same input to the same agent, staying
 * on this page; asks first when the run wrote, sent or delegated
 */
function RerunButton({ detail }: { detail: RunDetail }) {
  const { t } = useTranslation()
  const { run, agentName } = detail
  const rerun = () => {
    void sendAgentMessage(run.agentId, run.input).then(() =>
      toast.success(t('runs.rerunStarted', { agentName }), {
        action: {
          label: t('runDetail.rerun.viewRuns'),
          onClick: () =>
            useNavigationStore
              .getState()
              .navigate({ kind: 'agent', agentId: run.agentId, tab: 'runs' }),
        },
      })
    )
  }
  return (
    <ConfirmingButton
      className="border-primary/40 text-primary hover:bg-primary/10 hover:text-primary"
      onConfirm={rerun}
      confirm={
        rerunNeedsConfirm(detail)
          ? {
              title: t('runDetail.rerun.confirmTitle', { agentName }),
              description: t('runDetail.rerun.confirmDescription'),
              action: t('runDetail.rerun.confirmAction'),
            }
          : null
      }
    >
      <RotateCcw />
      {t('runs.rerun')}
    </ConfirmingButton>
  )
}

/** "Continued from run X": this run picks up a compacted run */
function ContinuedFrom({ parentRunId }: { parentRunId: string }) {
  const { t } = useTranslation()
  return (
    <div className="flex items-center gap-1 ps-9 text-xs text-muted-foreground">
      <CornerLeftUp className="size-3.5 shrink-0" aria-hidden />
      <Button
        variant="link"
        size="sm"
        className="h-auto p-0 text-xs"
        title={parentRunId}
        onClick={() =>
          useNavigationStore
            .getState()
            .navigate({ kind: 'run-detail', runId: parentRunId })
        }
      >
        {t('runs.continuedFrom', { id: shortId(parentRunId) })}
      </Button>
    </div>
  )
}

/** "Part of: <Request>": the delegated task this turn served */
function PartOfRequest({ runId }: { runId: string }) {
  const { t } = useTranslation()
  const { data: requestId } = useRequestForRun(runId)
  if (!requestId) return null
  return (
    <div className="flex items-center gap-1 ps-9 text-xs text-muted-foreground">
      <CornerLeftUp className="size-3.5 shrink-0" aria-hidden />
      <Button
        variant="link"
        size="sm"
        className="h-auto p-0 text-xs"
        onClick={() =>
          useNavigationStore.getState().navigate({ kind: 'request', requestId })
        }
      >
        {t('request.partOf')}
      </Button>
    </div>
  )
}

/** What the Request this run served asked, word for word */
function RunAsk({ runId }: { runId: string }) {
  const { data: requestId } = useRequestForRun(runId)
  if (!requestId) return null
  return <RequestAsk requestId={requestId} />
}

function RequestAsk({ requestId }: { requestId: string }) {
  const { data: request } = useRequest(requestId)
  return request ? <AskedBlock request={request} /> : null
}

/** The summary a compacted run's child started from, collapsed by default */
function CompactedContext({ summary }: { summary: string }) {
  const { t } = useTranslation()
  return (
    <Collapsible
      className="rounded-lg border"
      aria-label={t('runs.compactionSummaryTitle')}
    >
      <CollapsibleTrigger className="group flex w-full items-center gap-2 px-3 py-2 text-start text-sm font-medium">
        <ChevronRight className="size-4 transition-transform group-data-[state=open]:rotate-90 rtl:rotate-180 rtl:group-data-[state=open]:rotate-90" />
        <Layers className="size-4 text-muted-foreground" aria-hidden />
        {t('runs.compactionSummaryTitle')}
      </CollapsibleTrigger>
      <CollapsibleContent className="flex flex-col gap-2 px-3 pb-3">
        <p className="text-xs text-muted-foreground">
          {t('runs.compactionSummaryHint')}
        </p>
        <div className="text-sm">
          <MarkdownContent content={summary} />
        </div>
      </CollapsibleContent>
    </Collapsible>
  )
}

function RunDetailLayout({
  detail,
  initialTab,
}: {
  detail: RunDetail
  initialTab?: RunDetailTab
}) {
  const { t, i18n } = useTranslation()
  const { run } = detail
  const agent = displayAgent(run.agentId, detail.agentName)
  const active = isActive(run.status)
  // Following a run that's going: its timeline; once ended: the result
  const [tab, setTab] = useState<RunDetailTab>(
    () => initialTab ?? (active ? 'timeline' : 'overview')
  )
  const [highlightEventId, setHighlightEventId] = useState<string | null>(null)
  const now = useNow(active)
  // The summary is generated only once the Summary tab is opened
  const analysis = useRunAnalysis(run.id, { enabled: tab === 'summary' })

  // Ticks while the run is going; unknown for one that never ended
  const end = run.endedAt ?? (active ? now : null)
  const duration =
    end === null ? '—' : formatDuration(Math.max(0, end - run.startedAt))
  const startedAt = new Date(run.startedAt).toLocaleString(i18n.language, {
    dateStyle: 'medium',
    timeStyle: 'medium',
  })
  const startedShort = new Date(run.startedAt).toLocaleTimeString(
    i18n.language,
    { hour: '2-digit', minute: '2-digit' }
  )
  const approvals = detail.messages.filter(m => m.kind === 'approval').length
  const title = [detail.agentName, inputHeadline(run.input), startedShort]
    .filter(Boolean)
    .join(' · ')

  const openSource = (source: SourceRef) => {
    if (source.kind === 'event') {
      setHighlightEventId(source.id)
      setTab('logs')
    } else if (source.kind === 'artifact') {
      setTab('files')
    } else {
      setTab('timeline')
    }
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex shrink-0 flex-col gap-1.5 border-b bg-card px-4 py-4">
        <PartOfRequest runId={run.id} />
        {run.parentRunId && <ContinuedFrom parentRunId={run.parentRunId} />}
        <div className="flex flex-wrap items-center gap-2">
          <Button
            size="icon-sm"
            variant="ghost"
            onClick={() => useNavigationStore.getState().goBack()}
            aria-label={t('views.back')}
            title={t('views.back')}
          >
            <ArrowLeft className="rtl:rotate-180" />
          </Button>
          <AgentAvatar agent={agent} className="size-6" />
          <h1
            className="min-w-0 truncate text-lg font-semibold tracking-tight"
            title={`${detail.agentName} · ${run.input}`}
          >
            {title}
          </h1>
          <Badge variant="outline" className={RUN_STATUS_CLASSES[run.status]}>
            {t(`runs.status.${run.status}`)}
          </Badge>
          <div className="ms-auto flex items-center gap-2">
            {active ? (
              <StopButton detail={detail} />
            ) : (
              <RerunButton detail={detail} />
            )}
            <RunActions detail={detail} />
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 ps-9 text-xs text-muted-foreground">
          <span>
            {t('runDetail.header.started')}{' '}
            <time dateTime={new Date(run.startedAt).toISOString()}>
              {startedAt}
            </time>
          </span>
          <span aria-hidden>·</span>
          <span className="tabular-nums">
            {t('runDetail.header.duration')} {duration}
          </span>
          {run.model && (
            <>
              <span aria-hidden>·</span>
              <code dir="ltr" className="font-mono">
                {run.model}
              </code>
            </>
          )}
        </div>
      </header>

      <Tabs
        value={tab}
        onValueChange={value => setTab(value as RunDetailTab)}
        className="flex min-h-0 flex-1 flex-col gap-0"
      >
        <div className="shrink-0 overflow-x-auto border-b bg-card px-4">
          <TabsList variant="line">
            {RUN_DETAIL_TABS.map(value => (
              <TabsTrigger key={value} value={value}>
                {t(`runDetail.tabs.${value}`)}
                {value === 'approvals' && approvals > 0 && (
                  <span className="ms-1 rounded-full bg-warning/15 px-1.5 text-xs font-semibold text-warning tabular-nums">
                    {approvals}
                  </span>
                )}
              </TabsTrigger>
            ))}
          </TabsList>
        </div>
        <ScrollArea className="min-h-0 flex-1">
          <div className="p-4">
            <TabsContent value="overview" className="flex flex-col gap-4">
              <RunAsk runId={run.id} />
              <RunPlanCard runId={run.id} />
              {run.compactionSummary && (
                <CompactedContext summary={run.compactionSummary} />
              )}
              <RunKeyResults
                runId={run.id}
                onViewFiles={() => setTab('files')}
              />
              <RunMetadata detail={detail} duration={duration} />
            </TabsContent>
            <TabsContent value="summary">
              <RunSummaryTab analysis={analysis} onOpenSource={openSource} />
            </TabsContent>
            <TabsContent value="timeline">
              <RunTimeline runId={run.id} />
            </TabsContent>
            <TabsContent value="outputs">
              <RunOutputs detail={detail} />
            </TabsContent>
            <TabsContent value="files">
              <RunFiles detail={detail} />
            </TabsContent>
            <TabsContent value="logs">
              <RunLogs detail={detail} highlightEventId={highlightEventId} />
            </TabsContent>
            <TabsContent value="approvals">
              <RunApprovals detail={detail} />
            </TabsContent>
          </div>
        </ScrollArea>
        <RunAskBox ask={analysis.ask} onAsk={() => setTab('summary')} />
      </Tabs>
    </div>
  )
}
