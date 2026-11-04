import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Play, Workflow } from 'lucide-react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/ui/tooltip'
import {
  NewWorkflowMenu,
  WorkflowsEmptyActions,
} from '@/components/automations/NewWorkflowMenu'
import {
  currentStepNumber,
  failedStep,
  isActive,
  startedByText,
  stepLabel,
  triggersFor,
} from '@/components/automations/automations-model'
import { useWorkflow, useWorkflows } from '@/hooks/useSnapshots'
import { useTriggers } from '@/hooks/useTriggers'
import { formatDuration, formatRelativeTime } from '@/lib/format-time'
import { WORKFLOW_RUN_STATUS_CLASSES } from '@/lib/run-status'
import type { Trigger, WorkflowSummary } from '@/lib/tauri-bindings'
import { runWorkflow } from '@/lib/workflow-actions'
import { useNavigationStore } from '@/store/navigation-store'
import { ViewHeader } from './ViewHeader'

/** Every workflow in the agents repo: what starts it, its last run, Run. */
export function WorkflowsListView() {
  const { t } = useTranslation()
  const { data: workflows, isLoading, error, refetch } = useWorkflows()
  const { data: triggers } = useTriggers()
  const [search, setSearch] = useState('')
  const query = search.trim().toLowerCase()
  const shown = (workflows ?? []).filter(
    workflow => !query || workflow.name.toLowerCase().includes(query)
  )

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        title={t('nav.workflows')}
        subtitle={t('workflows.subtitle')}
        actions={<NewWorkflowMenu />}
      />
      <ScrollArea className="min-h-0 flex-1">
        <div className="mx-auto flex w-full max-w-7xl flex-col gap-4 p-6">
          {isLoading ? (
            <div className="flex flex-col gap-2">
              {[0, 1, 2].map(i => (
                <Skeleton key={i} className="h-12 rounded-md" />
              ))}
            </div>
          ) : error ? (
            <div className="flex items-center gap-3 text-sm text-destructive">
              {t('workflows.loadError')}
              <Button
                size="sm"
                variant="outline"
                onClick={() => void refetch()}
              >
                {t('automations.tryAgain')}
              </Button>
            </div>
          ) : !workflows || workflows.length === 0 ? (
            <Empty className="py-16">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <Workflow />
                </EmptyMedia>
                <EmptyTitle>{t('workflows.emptyTitle')}</EmptyTitle>
                <EmptyDescription>
                  {t('workflows.emptyDescription')}
                </EmptyDescription>
              </EmptyHeader>
              <EmptyContent>
                <WorkflowsEmptyActions />
              </EmptyContent>
            </Empty>
          ) : (
            <>
              <Input
                type="search"
                className="max-w-xs"
                value={search}
                onChange={event => setSearch(event.target.value)}
                placeholder={t('automations.search')}
                aria-label={t('automations.search')}
              />
              <div className="overflow-x-auto rounded-lg border">
                <table className="w-full text-sm">
                  <thead className="bg-muted/40 text-xs text-muted-foreground">
                    <tr className="[&_th]:px-3 [&_th]:py-2 [&_th]:text-start [&_th]:font-medium">
                      <th>{t('automations.columns.workflow')}</th>
                      <th>{t('automations.columns.steps')}</th>
                      <th>{t('automations.columns.startedBy')}</th>
                      <th>{t('automations.columns.lastRun')}</th>
                      <th>{t('automations.columns.status')}</th>
                      <th className="sr-only">
                        {t('automations.columns.run')}
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {shown.map(workflow => (
                      <WorkflowRow
                        key={workflow.id}
                        workflow={workflow}
                        triggers={triggersFor(workflow.id, triggers)}
                      />
                    ))}
                  </tbody>
                </table>
              </div>
            </>
          )}
        </div>
      </ScrollArea>
    </div>
  )
}

function stepCount(definition: WorkflowSummary['definition']): number {
  const steps =
    definition && typeof definition === 'object' && !Array.isArray(definition)
      ? definition.steps
      : undefined
  return Array.isArray(steps) ? steps.length : 0
}

function WorkflowRow({
  workflow,
  triggers,
}: {
  workflow: WorkflowSummary
  triggers: Trigger[]
}) {
  const { t, i18n } = useTranslation()
  // The page's runs (newest first) give the last-run summary
  const { data: detail } = useWorkflow(workflow.id)
  const lastRun = detail?.runs[0]
  const active = isActive(lastRun)
  const failed = failedStep(lastRun)

  const open = () =>
    useNavigationStore
      .getState()
      .navigate({ kind: 'workflow', workflowId: workflow.id })

  return (
    <tr className="border-t hover:bg-accent/40 [&_td]:px-3 [&_td]:py-2">
      <td>
        <button
          type="button"
          className="text-start font-medium hover:underline"
          onClick={open}
        >
          {workflow.name}
        </button>
      </td>
      <td className="tabular-nums">{stepCount(workflow.definition)}</td>
      <td className="text-xs">{startedByText(triggers, t, i18n.language)}</td>
      <td className="text-xs whitespace-nowrap text-muted-foreground">
        {lastRun ? (
          <>
            {formatRelativeTime(lastRun.startedAt, i18n.language)}
            {lastRun.endedAt !== null &&
              ` · ${formatDuration(lastRun.endedAt - lastRun.startedAt)}`}
            {active && (
              <span className="block">
                {t('automations.stepOf', {
                  current: currentStepNumber(lastRun),
                  total: lastRun.steps.length,
                })}
              </span>
            )}
          </>
        ) : (
          t('workflows.neverRun')
        )}
      </td>
      <td>
        {lastRun ? (
          <Badge
            variant="outline"
            className={WORKFLOW_RUN_STATUS_CLASSES[lastRun.status]}
          >
            {failed
              ? t('automations.failedAt', { step: stepLabel(failed.stepId) })
              : t(`workflows.status.${lastRun.status}`)}
          </Badge>
        ) : (
          <span className="text-xs text-muted-foreground">—</span>
        )}
      </td>
      <td className="text-end">
        <Tooltip>
          <TooltipTrigger asChild>
            <span>
              <Button
                size="sm"
                variant="ghost"
                disabled={active}
                onClick={() => void runWorkflow(workflow.id)}
                aria-label={t('workflows.runNamed', { name: workflow.name })}
              >
                <Play />
                {t('workflows.run')}
              </Button>
            </span>
          </TooltipTrigger>
          {active && (
            <TooltipContent>{t('automations.alreadyRunning')}</TooltipContent>
          )}
        </Tooltip>
      </td>
    </tr>
  )
}
