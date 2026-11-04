import type { TFunction } from 'i18next'
import { useTranslation } from 'react-i18next'
import {
  CalendarClock,
  CircleCheck,
  CircleSlash,
  Hand,
  Pause,
  Play,
  Square,
  TriangleAlert,
  Workflow,
} from 'lucide-react'
import { toast } from 'sonner'
import { Alert, AlertDescription } from '@/components/ui/alert'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { AskBox } from '@/components/ask/AskBox'
import { ViewHeader } from '@/components/views/ViewHeader'
import { WorkflowCanvas } from '@/components/workflow-builder'
import { useNow } from '@/components/agents/task-utils'
import { useAgentNames } from '@/hooks/useHome'
import { useWorkflow } from '@/hooks/useSnapshots'
import { useWorkAction, useWorkItem } from '@/hooks/useWork'
import type { WorkItem, WorkflowRun } from '@/lib/bindings'
import { openEditInStudio } from '@/lib/ask'
import { formatDuration, formatRelativeTime } from '@/lib/format-time'
import { cn } from '@/lib/utils'
import {
  comingUp,
  estimatedFinish,
  nextActionMessage,
  runControls,
  stepLabel,
  timeline,
  workItemView,
  type TimelineKind,
} from '@/lib/work'
import { useNavigationStore } from '@/store/navigation-store'
import { WorkProgressBar } from './WorkItemCard'

/**
 * One workflow run as a project: header (owner, started, ETA, progress,
 * pause / resume / cancel), the workflow's graph, a timeline of what
 * happened, what's coming up, and an ask box scoped to the run.
 */
export function WorkflowProjectView({ workId }: { workId: string }) {
  const { t } = useTranslation()
  const { data: item, error, isPending } = useWorkItem(workId)

  if (isPending) {
    return (
      <div className="flex flex-col gap-3 p-4">
        <Skeleton className="h-8 w-1/3" />
        <Skeleton className="h-40 w-full" />
      </div>
    )
  }
  if (error || !item) {
    return (
      <Alert variant="destructive" className="m-4">
        <AlertDescription>
          {error instanceof Error ? error.message : t('work.notFound')}
        </AlertDescription>
      </Alert>
    )
  }
  if (item.kind !== 'workflow' || !item.workflowId) {
    // Delegated tasks have their own thread view
    return (
      <div className="p-4">
        <Button
          onClick={() =>
            useNavigationStore.getState().navigate(workItemView(item))
          }
        >
          {t('work.openThread')}
        </Button>
      </div>
    )
  }
  return <Project item={item} workflowId={item.workflowId} />
}

function Project({ item, workflowId }: { item: WorkItem; workflowId: string }) {
  const { t, i18n } = useTranslation()
  const { data: workflow } = useWorkflow(workflowId)
  const run = workflow?.runs.find(r => r.id === item.id) ?? null
  const now = useNow(item.state === 'active')
  const action = useWorkAction()
  const agentName = useAgentNames()
  const finish = run ? estimatedFinish(run, now) : null
  const controls = runControls(item, run)

  const act = (kind: 'pause' | 'resume' | 'cancel') =>
    action.mutate(
      { id: item.id, action: kind },
      {
        onError: e => toast.error(e instanceof Error ? e.message : String(e)),
      }
    )

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        leading={
          <Workflow aria-hidden className="size-4 text-muted-foreground" />
        }
        title={item.title}
        titleAdornment={
          <Badge variant="outline">{t(`work.state.${item.state}`)}</Badge>
        }
        subtitle={[
          t('work.owner', { owner: item.owner ?? t('work.you') }),
          t('work.started', {
            when: formatRelativeTime(item.startedAt, i18n.language),
          }),
          finish !== null &&
            t('work.eta', { duration: formatDuration(finish - now) }),
        ]
          .filter(Boolean)
          .join(' · ')}
        actions={
          <div className="flex gap-1.5">
            {controls.pause && (
              <Button
                size="sm"
                variant="outline"
                disabled={action.isPending}
                onClick={() => act('pause')}
              >
                <Pause />
                {t('work.pause')}
              </Button>
            )}
            {controls.resume && (
              <Button
                size="sm"
                variant="outline"
                disabled={action.isPending}
                onClick={() => act('resume')}
              >
                <Play />
                {t('work.resume')}
              </Button>
            )}
            {controls.cancel && (
              <Button
                size="sm"
                variant="outline"
                disabled={action.isPending}
                onClick={() => act('cancel')}
              >
                <Square />
                {t('work.cancel')}
              </Button>
            )}
          </div>
        }
      />
      <div className="flex shrink-0 flex-col gap-2 border-b px-4 py-3">
        <WorkProgressBar item={item} />
        {item.nextAction && (
          <p
            className={cn(
              'text-sm',
              item.nextAction.kind === 'auto'
                ? 'text-muted-foreground'
                : 'text-warning'
            )}
          >
            {nextActionText(run, item.nextAction, agentName, t)}
          </p>
        )}
      </div>
      <div className="grid min-h-0 flex-1 grid-cols-1 overflow-y-auto lg:grid-cols-5">
        <section
          aria-label={t('work.graph')}
          className="h-80 border-b lg:col-span-3 lg:h-full lg:border-e lg:border-b-0"
        >
          <WorkflowCanvas workflowId={workflowId} />
        </section>
        <div className="flex min-h-0 flex-col gap-5 p-4 lg:col-span-2">
          {run ? (
            <>
              <Timeline run={run} agentName={agentName} />
              <ComingUp run={run} now={now} agentName={agentName} />
            </>
          ) : (
            <p className="text-sm text-muted-foreground">
              {t('work.runNotLoaded')}
            </p>
          )}
        </div>
      </div>
      <div className="shrink-0 border-t p-3">
        <AskBox
          context={{
            currentView: `workflow:${workflowId}`,
            focusedArtifact: { kind: 'workflow', id: workflowId },
            recentRuns: [item.id],
          }}
          placeholder={t('ask.placeholder.workflowRun')}
          onApplyEdit={openEditInStudio}
        />
      </div>
    </div>
  )
}

function nextActionText(
  run: WorkflowRun | null,
  next: NonNullable<WorkItem['nextAction']>,
  agentName: (id: string) => string,
  t: TFunction
): string {
  const message = nextActionMessage(run, next, agentName)
  return 'text' in message
    ? message.text
    : t(`work.next.${message.key}`, {
        step: message.step,
        agent: message.agent,
      })
}

const TIMELINE_ICONS: Record<TimelineKind, typeof CircleCheck> = {
  runStarted: Play,
  stepStarted: CalendarClock,
  stepDone: CircleCheck,
  stepFailed: TriangleAlert,
  stepRejected: CircleSlash,
  runEnded: CircleCheck,
}

function Timeline({
  run,
  agentName,
}: {
  run: WorkflowRun
  agentName: (id: string) => string
}) {
  const { t, i18n } = useTranslation()
  const events = timeline(run)
  return (
    <section aria-label={t('work.timeline')} className="flex flex-col gap-2">
      <h2 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        {t('work.timeline')}
      </h2>
      <ol className="flex flex-col gap-2 border-s ps-3">
        {events.map((event, index) => {
          const Icon = TIMELINE_ICONS[event.kind]
          return (
            <li key={index} className="flex items-start gap-2 text-sm">
              <Icon
                aria-hidden
                className={cn(
                  'mt-0.5 size-4 shrink-0',
                  event.kind === 'stepFailed' || event.kind === 'stepRejected'
                    ? 'text-destructive'
                    : 'text-muted-foreground'
                )}
              />
              <span className="min-w-0 flex-1">
                {t(`work.event.${event.kind}`, {
                  step: event.stepId ? stepLabel(event.stepId) : '',
                  agent: event.agentId ? agentName(event.agentId) : '',
                })}
                {event.detail && (
                  <span className="block truncate text-xs text-muted-foreground">
                    {event.detail}
                  </span>
                )}
              </span>
              <span className="shrink-0 text-xs text-muted-foreground">
                {formatRelativeTime(event.at, i18n.language)}
              </span>
            </li>
          )
        })}
      </ol>
    </section>
  )
}

function ComingUp({
  run,
  now,
  agentName,
}: {
  run: WorkflowRun
  now: number
  agentName: (id: string) => string
}) {
  const { t } = useTranslation()
  const upcoming = comingUp(run, now)
  return (
    <section aria-label={t('work.comingUp')} className="flex flex-col gap-2">
      <h2 className="text-xs font-semibold tracking-wide text-muted-foreground uppercase">
        {t('work.comingUp')}
      </h2>
      {upcoming.length === 0 ? (
        <p className="text-sm text-muted-foreground">{t('work.nothingNext')}</p>
      ) : (
        <ul className="flex flex-col gap-1.5">
          {upcoming.map(({ step, expectedAt, needsApproval }) => (
            <li key={step.stepId} className="flex items-center gap-2 text-sm">
              {needsApproval ? (
                <Hand aria-hidden className="size-4 shrink-0 text-warning" />
              ) : (
                <CalendarClock
                  aria-hidden
                  className="size-4 shrink-0 text-muted-foreground"
                />
              )}
              <span className="min-w-0 flex-1 truncate">
                {t('work.upcomingStep', {
                  step: stepLabel(step.stepId),
                  agent: agentName(step.agentId),
                })}
              </span>
              <span className="shrink-0 text-xs text-muted-foreground">
                {needsApproval
                  ? t('work.needsApproval')
                  : t('work.inAbout', {
                      duration: formatDuration(Math.max(0, expectedAt - now)),
                    })}
              </span>
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
