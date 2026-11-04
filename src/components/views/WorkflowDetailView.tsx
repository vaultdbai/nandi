import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  Bot,
  Clock,
  ExternalLink,
  MoreHorizontal,
  Pencil,
  Play,
  Plus,
  RotateCcw,
  ShieldAlert,
  TriangleAlert,
  UserRound,
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
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import {
  Empty,
  EmptyContent,
  EmptyHeader,
  EmptyTitle,
} from '@/components/ui/empty'
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
} from '@/components/ui/resizable'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Textarea } from '@/components/ui/textarea'
import { MarkdownContent } from '@/components/agent-console/MarkdownContent'
import {
  currentStepNumber,
  describeSchedule,
  failedStep,
  isActive,
  runRequestView,
  runStarter,
  startedByText,
  stepLabel,
  triggersFor,
  waitingStep,
  workflowHealth,
} from '@/components/automations/automations-model'
import {
  MarkStepDoneButton,
  StepApprovalButtons,
} from '@/components/automations/StepActions'
import { NewTriggerDialog } from '@/components/triggers/NewTriggerDialog'
import {
  StepDetailsPanel,
  WorkflowCanvas,
  saveWorkflowYaml,
} from '@/components/workflow-builder'
import {
  StepSegments,
  WorkflowRunSummary,
} from '@/components/workflow-builder/run-display'
import {
  STEP_STATUS_STYLES,
  stepDuration,
} from '@/components/workflow-builder/run-status-styles'
import { useNow } from '@/components/agents/task-utils'
import { useAgents } from '@/hooks/useAgents'
import { useWorkflow } from '@/hooks/useSnapshots'
import { useCronPreview, useTriggers } from '@/hooks/useTriggers'
import { openWorkflowInStudio } from '@/lib/ask'
import { formatDuration, formatRelativeTime } from '@/lib/format-time'
import { WORKFLOW_RUN_STATUS_CLASSES } from '@/lib/run-status'
import type {
  Trigger,
  Workflow,
  WorkflowRun,
  WorkflowStepRun,
} from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'
import { runWorkflow } from '@/lib/workflow-actions'
import { useNavigationStore, type WorkflowTab } from '@/store/navigation-store'
import { ViewHeader } from './ViewHeader'
import { ErrorMessage } from '@/components/ui/error-message'

const navigate = useNavigationStore.getState().navigate

/**
 * One workflow: what it does (Overview), its steps (Graph, with the step
 * panel), past runs (Runs), and its file (Configuration). A step waiting
 * on the user shows one banner by name, with its fix.
 */
export function WorkflowDetailView({
  workflowId,
  initialTab = 'overview',
}: {
  workflowId: string
  initialTab?: WorkflowTab
}) {
  const { t } = useTranslation()
  const { data: workflow, isLoading, error } = useWorkflow(workflowId)
  const [tab, setTab] = useState<WorkflowTab>(initialTab)
  const [selectedStepId, setSelectedStepId] = useState<string | null>(null)
  // Which run the Runs tab shows; the latest by default
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null)
  // The file draft lives here so switching tabs keeps it
  const [draft, setDraft] = useState<string | null>(null)
  const dirty =
    workflow !== undefined && draft !== null && draft !== workflow.source

  useEffect(() => {
    useNavigationStore.getState().setLeaveGuarded(dirty)
  }, [dirty])
  useEffect(
    () => () => useNavigationStore.getState().setLeaveGuarded(false),
    []
  )

  if (isLoading) {
    return (
      <div className="flex flex-col gap-4 p-6">
        <Skeleton className="h-10 w-1/2" />
        <Skeleton className="h-8 w-80" />
        <Skeleton className="h-48" />
      </div>
    )
  }
  if (error || !workflow) {
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyTitle>{t('automations.workflowGone')}</EmptyTitle>
        </EmptyHeader>
        <EmptyContent>
          <Button
            onClick={() => navigate({ kind: 'automations', tab: 'workflows' })}
          >
            {t('automations.backToAutomations')}
          </Button>
        </EmptyContent>
      </Empty>
    )
  }

  const latest = workflow.runs[0]
  const selectedRun =
    workflow.runs.find(run => run.id === selectedRunId) ?? latest

  return (
    <div className="flex h-full min-h-0 flex-col">
      <WorkflowHeader workflow={workflow} latest={latest} />
      <WorkflowBanner workflow={workflow} run={latest} />
      <Tabs
        value={tab}
        onValueChange={value => setTab(value as WorkflowTab)}
        className="min-h-0 flex-1 gap-0"
      >
        <div className="border-b px-4">
          <TabsList variant="line">
            <TabsTrigger value="overview">
              {t('automations.tabs.overview')}
            </TabsTrigger>
            <TabsTrigger value="graph">
              {t('workflowBuilder.tabs.graph')}
            </TabsTrigger>
            <TabsTrigger value="runs">
              {t('workflowBuilder.tabs.runs')}
            </TabsTrigger>
            <TabsTrigger value="configuration">
              {t('workflowBuilder.tabs.configuration')}
              {dirty && (
                <span
                  aria-label={t('automations.unsaved')}
                  className="ms-1 size-1.5 rounded-full bg-warning"
                />
              )}
            </TabsTrigger>
          </TabsList>
        </div>
        <TabsContent value="overview" className="min-h-0 flex-1">
          <OverviewTab workflow={workflow} onAllRuns={() => setTab('runs')} />
        </TabsContent>
        <TabsContent value="graph" className="min-h-0 flex-1">
          {/* The step panel shows only here, and only for a selected step */}
          <ResizablePanelGroup direction="horizontal" className="h-full">
            <ResizablePanel
              defaultSize={selectedStepId ? 65 : 100}
              minSize={35}
            >
              <WorkflowCanvas
                workflowId={workflow.id}
                selectedStepId={selectedStepId}
                onSelectStep={setSelectedStepId}
              />
            </ResizablePanel>
            {selectedStepId && (
              <>
                <ResizableHandle withHandle />
                <ResizablePanel defaultSize={35} minSize={20}>
                  <StepDetailsPanel
                    workflowId={workflow.id}
                    stepId={selectedStepId}
                    run={latest}
                    onClose={() => setSelectedStepId(null)}
                    onStepRenamed={setSelectedStepId}
                  />
                </ResizablePanel>
              </>
            )}
          </ResizablePanelGroup>
        </TabsContent>
        <TabsContent value="runs" className="min-h-0 flex-1">
          <RunsTab
            workflow={workflow}
            selectedRun={selectedRun}
            onSelect={setSelectedRunId}
          />
        </TabsContent>
        <TabsContent value="configuration" className="min-h-0 flex-1">
          <ConfigurationTab
            workflow={workflow}
            draft={draft ?? workflow.source}
            onDraft={setDraft}
            onSaved={() => setDraft(null)}
          />
        </TabsContent>
      </Tabs>
    </div>
  )
}

function WorkflowHeader({
  workflow,
  latest,
}: {
  workflow: Workflow
  latest: WorkflowRun | undefined
}) {
  const { t, i18n } = useTranslation()
  const { data: triggers } = useTriggers()
  const starters = triggersFor(workflow.id, triggers)
  const [addingTrigger, setAddingTrigger] = useState(false)
  const failed = failedStep(latest)
  const subtitle = [
    t('workflows.stepCount', { count: workflow.steps.length }),
    t('automations.startedByLine', {
      by: startedByText(starters, t, i18n.language),
    }),
    latest
      ? t('workflows.lastRun', {
          time: formatRelativeTime(latest.startedAt, i18n.language),
        })
      : t('workflows.neverRun'),
  ].join(' · ')

  return (
    <>
      <ViewHeader
        title={workflow.name}
        titleAdornment={
          latest && (
            <Badge
              variant="outline"
              className={WORKFLOW_RUN_STATUS_CLASSES[latest.status]}
            >
              {failed
                ? t('automations.failedAt', { step: stepLabel(failed.stepId) })
                : t(`workflows.status.${latest.status}`)}
            </Badge>
          )
        }
        subtitle={subtitle}
        actions={
          <>
            <Button
              size="sm"
              variant="outline"
              onClick={() => openWorkflowInStudio(workflow.id)}
            >
              <Pencil />
              {t('automations.editInStudio')}
            </Button>
            <Button
              size="sm"
              disabled={isActive(latest)}
              title={
                isActive(latest) ? t('automations.alreadyRunning') : undefined
              }
              onClick={() => void runWorkflow(workflow.id)}
            >
              <Play />
              {t('workflows.run')}
            </Button>
            <DropdownMenu>
              <DropdownMenuTrigger asChild>
                <Button
                  size="icon-sm"
                  variant="ghost"
                  aria-label={t('automations.workflowMenu', {
                    name: workflow.name,
                  })}
                >
                  <MoreHorizontal />
                </Button>
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end">
                <DropdownMenuItem onSelect={() => setAddingTrigger(true)}>
                  {t('automations.addTrigger')}
                </DropdownMenuItem>
                <DropdownMenuItem
                  onSelect={() =>
                    void navigator.clipboard
                      .writeText(workflow.id)
                      .then(() => toast.success(t('automations.idCopied')))
                  }
                >
                  {t('automations.copyId')}
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </>
        }
      />
      <NewTriggerDialog
        open={addingTrigger}
        onOpenChange={setAddingTrigger}
        initial={{ targetType: 'workflow', targetId: workflow.id }}
      />
    </>
  )
}

/**
 * One banner: the oldest step of the latest run waiting on the user (by
 * step name and agent, never its ID), or where a failed run stopped
 */
function WorkflowBanner({
  workflow,
  run,
}: {
  workflow: Workflow
  run: WorkflowRun | undefined
}) {
  const { t } = useTranslation()
  const { data: agents } = useAgents()
  const waiting = waitingStep(run)
  const failed = failedStep(run)
  const agentName = (id: string) =>
    agents?.find(agent => agent.id === id)?.name ?? id
  if (!run) return null

  if (waiting) {
    const manual = waiting.status === 'awaitingInput'
    return (
      <div className="flex flex-wrap items-center gap-3 border-b border-warning/30 bg-warning/10 px-4 py-2 text-sm">
        {manual ? (
          <UserRound aria-hidden className="size-4 text-warning" />
        ) : (
          <ShieldAlert aria-hidden className="size-4 text-warning" />
        )}
        <span className="min-w-0 flex-1">
          {manual
            ? t('automations.banner.manual', {
                step: stepLabel(waiting.stepId),
              })
            : t('automations.banner.approval', {
                step: stepLabel(waiting.stepId),
                agent: agentName(waiting.agentId),
              })}
        </span>
        <Button
          size="sm"
          variant="ghost"
          onClick={() => navigate(runRequestView(run))}
        >
          {t('automations.review')}
        </Button>
        {manual ? (
          <MarkStepDoneButton
            workflowId={workflow.id}
            runId={run.id}
            stepId={waiting.stepId}
          />
        ) : (
          <StepApprovalButtons
            workflowId={workflow.id}
            runId={run.id}
            stepId={waiting.stepId}
          />
        )}
      </div>
    )
  }

  if (failed) {
    return (
      <div className="flex flex-wrap items-center gap-3 border-b border-destructive/30 bg-destructive/5 px-4 py-2 text-sm">
        <TriangleAlert aria-hidden className="size-4 text-destructive" />
        <span className="min-w-0 flex-1">
          {t('automations.banner.failed', { step: stepLabel(failed.stepId) })}
          {(failed.error ?? run.error) && (
            <span className="ms-1 text-muted-foreground">
              {(failed.error ?? run.error ?? '').split('\n')[0]}
            </span>
          )}
        </span>
        <Button
          size="sm"
          variant="outline"
          onClick={() => navigate(runRequestView(run))}
        >
          {t('automations.viewProgress')}
        </Button>
        <RunAgainButton workflow={workflow} />
      </div>
    )
  }
  return null
}

/** Run again repeats every step, so it asks first */
function RunAgainButton({ workflow }: { workflow: Workflow }) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  return (
    <>
      <Button size="sm" variant="outline" onClick={() => setOpen(true)}>
        <RotateCcw />
        {t('automations.runAgain')}
      </Button>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t('automations.runAgainTitle')}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t('automations.runAgainDescription', {
                steps: workflow.steps
                  .map(step => stepLabel(step.id))
                  .join(', '),
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('automations.cancel')}</AlertDialogCancel>
            <AlertDialogAction onClick={() => void runWorkflow(workflow.id)}>
              {t('automations.runAgain')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  )
}

/** What it does, health, steps with agents, what starts it, recent runs */
function OverviewTab({
  workflow,
  onAllRuns,
}: {
  workflow: Workflow
  onAllRuns: () => void
}) {
  const { t, i18n } = useTranslation()
  const { data: agents } = useAgents()
  const { data: triggers } = useTriggers()
  const starters = triggersFor(workflow.id, triggers)
  // The 30-day window only needs a minute's precision
  const now = useNow(true, 60_000)
  const health = workflowHealth(workflow.runs, now)
  const [addingTrigger, setAddingTrigger] = useState(false)
  const agentName = (id: string) =>
    agents?.find(agent => agent.id === id)?.name ?? id

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto grid w-full max-w-6xl grid-cols-1 gap-4 p-4 lg:grid-cols-2">
        <Card className="gap-2">
          <CardHeader>
            <CardTitle>{t('automations.overview.whatItDoes')}</CardTitle>
          </CardHeader>
          <CardContent className="text-sm">
            {workflow.description ? (
              <p dir="auto">{workflow.description}</p>
            ) : (
              <p className="text-muted-foreground">
                {t('automations.overview.noDescription')}{' '}
                <Button
                  variant="link"
                  size="sm"
                  className="h-auto p-0"
                  onClick={() => openWorkflowInStudio(workflow.id)}
                >
                  {t('automations.overview.addInStudio')}
                </Button>
              </p>
            )}
          </CardContent>
        </Card>

        <Card className="gap-2">
          <CardHeader>
            <CardTitle>{t('automations.overview.health')}</CardTitle>
          </CardHeader>
          <CardContent className="text-sm">
            {health.total === 0 ? (
              <p className="text-muted-foreground">
                {t('automations.overview.noRuns')}
              </p>
            ) : (
              <p>
                {t('automations.overview.healthLine', {
                  total: health.total,
                  done: health.done,
                  failed: health.failed,
                  waiting: health.waiting,
                })}
                {health.avgMs !== null &&
                  ` · ${t('automations.overview.avg', {
                    duration: formatDuration(health.avgMs),
                  })}`}
              </p>
            )}
          </CardContent>
        </Card>

        <Card className="gap-2">
          <CardHeader>
            <CardTitle>{t('automations.overview.steps')}</CardTitle>
          </CardHeader>
          <CardContent>
            <ol className="flex flex-col gap-1.5 text-sm">
              {workflow.steps.map((step, index) => (
                <li key={step.id} className="flex items-center gap-2">
                  <span className="w-5 text-xs text-muted-foreground tabular-nums">
                    {index + 1}
                  </span>
                  <span className="font-medium">{stepLabel(step.id)}</span>
                  <span className="text-muted-foreground">
                    {step.manual
                      ? t('automations.step.byPerson')
                      : agentName(step.agent)}
                    {step.requiresApproval &&
                      ` · ${t('automations.step.asksFirst')}`}
                  </span>
                </li>
              ))}
            </ol>
          </CardContent>
        </Card>

        <Card className="gap-2">
          <CardHeader>
            <CardTitle>{t('automations.overview.startedBy')}</CardTitle>
          </CardHeader>
          <CardContent className="flex flex-col items-start gap-2 text-sm">
            {starters.length === 0 && (
              <p className="text-muted-foreground">
                {t('automations.overview.manualOnly')}
              </p>
            )}
            {starters.map(trigger => (
              <StarterRow
                key={trigger.id}
                trigger={trigger}
                language={i18n.language}
              />
            ))}
            <Button
              size="sm"
              variant="outline"
              onClick={() => setAddingTrigger(true)}
            >
              <Plus />
              {t('automations.addTrigger')}
            </Button>
            <NewTriggerDialog
              open={addingTrigger}
              onOpenChange={setAddingTrigger}
              initial={{ targetType: 'workflow', targetId: workflow.id }}
            />
          </CardContent>
        </Card>

        <Card className="gap-2 lg:col-span-2">
          <CardHeader className="flex flex-row items-center justify-between">
            <CardTitle>{t('automations.overview.recentRuns')}</CardTitle>
            {workflow.runs.length > 0 && (
              <Button size="sm" variant="ghost" onClick={onAllRuns}>
                {t('automations.overview.allRuns')}
              </Button>
            )}
          </CardHeader>
          <CardContent>
            {workflow.runs.length === 0 ? (
              <div className="flex items-center gap-3 text-sm text-muted-foreground">
                {t('automations.runs.empty')}
                <Button size="sm" onClick={() => void runWorkflow(workflow.id)}>
                  <Play />
                  {t('automations.runNow')}
                </Button>
              </div>
            ) : (
              <ul className="flex flex-col divide-y text-sm">
                {workflow.runs.slice(0, 5).map(run => (
                  <li key={run.id}>
                    <button
                      type="button"
                      className="flex w-full items-center gap-3 py-2 text-start hover:bg-accent/40"
                      onClick={() => navigate(runRequestView(run))}
                    >
                      <Badge
                        variant="outline"
                        className={WORKFLOW_RUN_STATUS_CLASSES[run.status]}
                      >
                        {t(`workflows.status.${run.status}`)}
                      </Badge>
                      <span className="text-xs text-muted-foreground">
                        {formatRelativeTime(run.startedAt, i18n.language)}
                      </span>
                      <span className="min-w-0 flex-1 truncate text-xs">
                        <RunStatusLine run={run} />
                      </span>
                      <span className="text-xs text-muted-foreground">
                        <StarterText run={run} />
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>
      </div>
    </ScrollArea>
  )
}

/** A trigger that starts this workflow, with when it next fires */
function StarterRow({
  trigger,
  language,
}: {
  trigger: Trigger
  language: string
}) {
  const { t } = useTranslation()
  const config = trigger.sourceConfig as { schedule?: unknown }
  const schedule =
    trigger.sourceType === 'cron' && typeof config?.schedule === 'string'
      ? config.schedule
      : ''
  const preview = useCronPreview(schedule)
  const next = preview.data?.valid ? preview.data.next[0] : undefined

  return (
    <button
      type="button"
      className="flex items-center gap-2 text-start hover:underline"
      onClick={() => navigate({ kind: 'trigger', triggerId: trigger.id })}
    >
      <Clock aria-hidden className="size-4 text-muted-foreground" />
      <span>
        {schedule
          ? (describeSchedule(schedule, language) ?? trigger.name)
          : trigger.name}
        {next !== undefined &&
          ` (${t('automations.nextRun', {
            time: new Date(next).toLocaleString(language, {
              weekday: 'short',
              hour: '2-digit',
              minute: '2-digit',
            }),
          })})`}
        {!trigger.enabled && ` · ${t('triggers.disabled')}`}
      </span>
    </button>
  )
}

/** "step 3 of 4 · waiting on you", "failed at Analyze", or the duration */
function RunStatusLine({ run }: { run: WorkflowRun }) {
  const { t } = useTranslation()
  const failed = failedStep(run)
  if (failed) {
    return <>{t('automations.failedAt', { step: stepLabel(failed.stepId) })}</>
  }
  if (isActive(run)) {
    return (
      <>
        {t('automations.stepOf', {
          current: currentStepNumber(run),
          total: run.steps.length,
        })}
        {waitingStep(run) && ` · ${t('automations.waitingOnYou')}`}
      </>
    )
  }
  return run.endedAt !== null ? (
    <>{formatDuration(run.endedAt - run.startedAt)}</>
  ) : null
}

/** Who started a run: a trigger by name, You, or another Nandi */
function StarterText({ run }: { run: WorkflowRun }) {
  const { t } = useTranslation()
  const { data: triggers } = useTriggers()
  const starter = runStarter(run.source ?? null, triggers)
  switch (starter.kind) {
    case 'manual':
      return <>{t('automations.from.you')}</>
    case 'trigger':
      return (
        <>
          {t('automations.from.trigger', {
            name: starter.name ?? t('automations.from.removedTrigger'),
          })}
        </>
      )
    case 'otherNandi':
      return <>{t('automations.from.otherNandi')}</>
    case 'unknown':
      return <>—</>
  }
}

function RunsTab({
  workflow,
  selectedRun,
  onSelect,
}: {
  workflow: Workflow
  selectedRun: WorkflowRun | undefined
  onSelect: (runId: string) => void
}) {
  const { t, i18n } = useTranslation()

  if (workflow.runs.length === 0) {
    return (
      <Empty className="h-full">
        <EmptyHeader>
          <EmptyTitle>{t('automations.runs.empty')}</EmptyTitle>
        </EmptyHeader>
        <EmptyContent>
          <Button onClick={() => void runWorkflow(workflow.id)}>
            <Play />
            {t('automations.runNow')}
          </Button>
        </EmptyContent>
      </Empty>
    )
  }

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-4 p-4">
        <div className="overflow-x-auto rounded-lg border">
          <table className="w-full text-sm">
            <thead className="bg-muted/40 text-xs text-muted-foreground">
              <tr className="[&_th]:px-3 [&_th]:py-2 [&_th]:text-start [&_th]:font-medium">
                <th>{t('automations.runs.run')}</th>
                <th>{t('automations.columns.startedBy')}</th>
                <th>{t('automations.columns.status')}</th>
                <th>{t('automations.runs.duration')}</th>
                <th>{t('automations.runs.request')}</th>
              </tr>
            </thead>
            <tbody>
              {workflow.runs.map(run => (
                <tr
                  key={run.id}
                  aria-selected={run.id === selectedRun?.id}
                  onClick={() => onSelect(run.id)}
                  className={cn(
                    'cursor-pointer border-t hover:bg-accent/40 [&_td]:px-3 [&_td]:py-2',
                    run.id === selectedRun?.id && 'bg-accent/60'
                  )}
                >
                  <td className="text-xs whitespace-nowrap">
                    {formatRelativeTime(run.startedAt, i18n.language)}
                  </td>
                  <td className="text-xs">
                    <StarterText run={run} />
                  </td>
                  <td className="text-xs">
                    <span className="flex flex-wrap items-center gap-2">
                      <Badge
                        variant="outline"
                        className={WORKFLOW_RUN_STATUS_CLASSES[run.status]}
                      >
                        {t(`workflows.status.${run.status}`)}
                      </Badge>
                      <RunStatusLine run={run} />
                    </span>
                  </td>
                  <td className="text-xs tabular-nums">
                    {run.endedAt !== null
                      ? formatDuration(run.endedAt - run.startedAt)
                      : '—'}
                  </td>
                  <td>
                    <Button
                      size="sm"
                      variant="link"
                      className="h-auto p-0 text-xs"
                      onClick={event => {
                        event.stopPropagation()
                        navigate(runRequestView(run))
                      }}
                    >
                      {t('automations.runs.open')}
                      <ExternalLink />
                    </Button>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {selectedRun && (
          <>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <p className="text-sm font-medium">
                {t('automations.runs.selected', {
                  time: formatRelativeTime(
                    selectedRun.startedAt,
                    i18n.language
                  ),
                })}
              </p>
              <div className="flex gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => navigate(runRequestView(selectedRun))}
                >
                  {t('automations.viewProgress')}
                </Button>
                <RunAgainButton workflow={workflow} />
              </div>
            </div>
            <WorkflowRunSummary run={selectedRun} />
            <StepSegments steps={selectedRun.steps} className="h-1.5" />
            <TranscriptCard workflow={workflow} run={selectedRun} />
          </>
        )}
      </div>
    </ScrollArea>
  )
}

/** The workflow file (advanced). The draft is the page's, so tabs keep it. */
function ConfigurationTab({
  workflow,
  draft,
  onDraft,
  onSaved,
}: {
  workflow: Workflow
  draft: string
  onDraft: (draft: string | null) => void
  onSaved: () => void
}) {
  const { t } = useTranslation()
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const changed = draft !== workflow.source

  const save = async () => {
    setSaving(true)
    setError(null)
    const failure = await saveWorkflowYaml(workflow.id, draft)
    setSaving(false)
    if (failure) setError(failure)
    else onSaved()
  }

  return (
    <div className="flex h-full min-h-0 flex-col gap-3 p-4">
      <div className="flex items-center justify-between">
        <label htmlFor="workflow-yaml" className="text-sm font-medium">
          {t('automations.workflowFile')}
        </label>
        {changed && (
          <span className="flex items-center gap-1.5 text-xs text-warning">
            <span className="size-1.5 rounded-full bg-warning" />
            {t('automations.unsaved')}
          </span>
        )}
      </div>
      <Textarea
        id="workflow-yaml"
        dir="ltr"
        spellCheck={false}
        className="min-h-64 flex-1 resize-none text-start font-mono text-xs [field-sizing:fixed]"
        value={draft}
        onChange={e => onDraft(e.target.value)}
      />
      {error && (
        <ErrorMessage
          error={error}
          detailsOpen
          className="rounded-md border border-destructive/30 bg-destructive/10 p-2"
        />
      )}
      <div className="flex justify-end gap-2">
        <Button
          size="sm"
          variant="outline"
          disabled={!changed || saving}
          onClick={() => {
            onDraft(null)
            setError(null)
          }}
        >
          {t('workflowBuilder.revert')}
        </Button>
        <Button
          size="sm"
          disabled={!changed || saving}
          onClick={() => void save()}
        >
          {t('workflowBuilder.save')}
        </Button>
      </div>
    </div>
  )
}

function TranscriptCard({
  workflow,
  run,
}: {
  workflow: Workflow
  run: WorkflowRun
}) {
  const { t } = useTranslation()

  return (
    <Card className="gap-4">
      <CardHeader>
        <CardTitle>{t('workflows.transcript')}</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-3">
        {run.error && (
          <p
            role="alert"
            className="rounded-lg border border-destructive/30 bg-destructive/10 p-3 text-sm text-destructive"
          >
            {run.error}
          </p>
        )}
        <ol className="flex flex-col">
          {run.steps.map((step, index) => (
            <StepTranscript
              key={step.stepId}
              workflowId={workflow.id}
              runId={run.id}
              step={step}
              index={index}
              last={index === run.steps.length - 1}
            />
          ))}
        </ol>
      </CardContent>
    </Card>
  )
}

/**
 * One step on the run's timeline: a status marker on the rail, then the
 * step's input, output, and (while waiting) its fix
 */
function StepTranscript({
  workflowId,
  runId,
  step,
  index,
  last,
}: {
  workflowId: string
  runId: string
  step: WorkflowStepRun
  index: number
  last: boolean
}) {
  const { t } = useTranslation()
  const { data: agents } = useAgents()
  const agentRunId = step.agentRunId
  const style = STEP_STATUS_STYLES[step.status]
  const Icon = style.icon
  const now = useNow(step.startedAt !== null && step.endedAt === null)
  const duration = stepDuration(step, now)
  const agentName =
    agents?.find(agent => agent.id === step.agentId)?.name ?? step.agentId

  return (
    <li className="relative flex gap-3 pb-4 last:pb-0">
      {!last && (
        <span
          aria-hidden
          className={cn(
            'absolute start-4 top-9 bottom-0 w-0.5 -translate-x-1/2 rounded-full rtl:translate-x-1/2',
            step.status === 'done' ? 'bg-success/60' : 'bg-border'
          )}
        />
      )}
      <span
        className={cn(
          'relative z-10 flex size-8 shrink-0 items-center justify-center rounded-full ring-4 ring-card',
          style.tileClassName
        )}
      >
        <Icon aria-hidden className={cn('size-4', style.iconClassName)} />
      </span>

      <div
        className={cn(
          'flex min-w-0 flex-1 flex-col gap-3 rounded-xl border bg-card p-3 shadow-xs',
          step.status === 'running' && 'border-info/40 shadow-info/10',
          step.status === 'awaitingApproval' && 'border-warning/40',
          (step.status === 'failed' || step.status === 'rejected') &&
            'border-destructive/40'
        )}
      >
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <span className="text-xs font-medium text-muted-foreground tabular-nums">
            {t('workflows.stepNumber', { number: index + 1 })}
          </span>
          <span className="font-semibold">{stepLabel(step.stepId)}</span>
          {step.manual ? (
            <span className="inline-flex items-center gap-1 rounded-md bg-muted px-1.5 py-0.5 text-xs">
              <UserRound aria-hidden className="size-3" />
              {t('automations.step.byPerson')}
            </span>
          ) : (
            <span className="inline-flex items-center gap-1 rounded-md bg-kind-agent/10 px-1.5 py-0.5 text-xs text-kind-agent">
              <Bot aria-hidden className="size-3" />
              {agentName}
            </span>
          )}
          <Badge variant="outline" className={style.pillClassName}>
            {t(`workflows.stepStatus.${step.status}`)}
          </Badge>
          <span className="ms-auto flex items-center gap-2">
            {duration !== null && (
              <span className="inline-flex items-center gap-1 text-xs text-muted-foreground tabular-nums">
                <Clock aria-hidden className="size-3" />
                {formatDuration(duration)}
              </span>
            )}
            {agentRunId && (
              <Button
                size="sm"
                variant="ghost"
                className="h-7"
                onClick={() =>
                  navigate({
                    kind: 'run',
                    agentId: step.agentId,
                    runId: agentRunId,
                  })
                }
              >
                <ExternalLink />
                {t('workflows.openRun')}
              </Button>
            )}
          </span>
        </div>

        {step.input !== null && (
          <section className="rounded-lg border-s-2 border-info/50 bg-muted/60 px-3 py-2">
            <h4 className="text-[11px] font-semibold tracking-wide text-info uppercase">
              {t('workflows.input')}
            </h4>
            <p dir="auto" className="text-sm break-words whitespace-pre-wrap">
              {step.input}
            </p>
          </section>
        )}
        {step.output !== null && (
          <section className="rounded-lg border-s-2 border-success/60 bg-success/5 px-3 py-2">
            <h4 className="text-[11px] font-semibold tracking-wide text-success uppercase">
              {t('workflows.output')}
            </h4>
            <MarkdownContent content={step.output} />
          </section>
        )}
        {step.error && (
          <p className="rounded-lg border-s-2 border-destructive/60 bg-destructive/5 px-3 py-2 text-sm text-destructive">
            {step.error}
          </p>
        )}

        {step.status === 'awaitingApproval' && (
          <StepApprovalButtons
            workflowId={workflowId}
            runId={runId}
            stepId={step.stepId}
          />
        )}
        {step.status === 'awaitingInput' && (
          <MarkStepDoneButton
            workflowId={workflowId}
            runId={runId}
            stepId={step.stepId}
          />
        )}
      </div>
    </li>
  )
}
