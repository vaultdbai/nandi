import { useTranslation } from 'react-i18next'
import { CircleCheck, TriangleAlert } from 'lucide-react'
import { ApprovalRequestCard } from '@/components/agent-console/ApprovalRequestCard'
import { TurnQueueStrip } from '@/components/agent-console/TurnQueue'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Skeleton } from '@/components/ui/skeleton'
import { useAgentDefinition } from '@/hooks/useAgentEditor'
import { useHarnesses } from '@/hooks/useHarnesses'
import { useDefaultModel } from '@/hooks/useLlmKeyStatus'
import { useRuns } from '@/hooks/useRuns'
import { useWorkflows } from '@/hooks/useSnapshots'
import { decideApproval } from '@/lib/agent-actions'
import { formatDuration } from '@/lib/format-time'
import { openSettings } from '@/lib/settings-nav'
import type { AgentOverview, ContainerState } from '@/lib/bindings'
import { usePendingApprovals } from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'
import type { AgentInfo } from '@/types/agent'
import { AgentHealthCells } from './AgentHealthCells'
import { AgentRunRow } from './AgentRunsTab'
import {
  hasModelFormatError,
  isCliRuntime,
  runtimeAuthMissing,
  runtimeName,
  type AgentTab,
} from './agent-page-model'

const RECENT = 5

/**
 * What this agent is doing and has done: its week, what waits on me, its
 * latest work, its queue, and what it's for.
 */
export function AgentOverviewTab({
  agent,
  overview,
  container,
  onOpenTab,
}: {
  agent: AgentInfo
  overview: AgentOverview | undefined
  container: ContainerState
  onOpenTab: (tab: AgentTab) => void
}) {
  const { t } = useTranslation()
  const pending = usePendingApprovals(agent.id)
  const recent = useRuns({ agentId: agent.id }, 0, RECENT)
  const definition = useAgentDefinition(agent.id)
  const allWorkflows = useWorkflows()
  const workflows = (definition.data?.workflows ?? []).map(id => ({
    id,
    name: allWorkflows.data?.find(w => w.id === id)?.name ?? id,
  }))

  return (
    <div className="mx-auto grid w-full max-w-5xl grid-cols-1 gap-4 p-4 lg:grid-cols-2">
      <HealthCard
        agent={agent}
        overview={overview}
        container={container}
        onOpenTab={onOpenTab}
      />

      <Card className="gap-3">
        <CardHeader>
          <CardTitle>
            {t('agentPage.overview.waiting', { count: pending.length })}
          </CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-2">
          {pending.length === 0 ? (
            <p className="flex items-center gap-2 text-sm text-muted-foreground">
              <CircleCheck aria-hidden className="size-4 text-success" />
              {t('agentPage.overview.nothingWaiting')}
            </p>
          ) : (
            pending.map(approval => (
              <ApprovalRequestCard
                key={approval.id}
                message={{
                  id: approval.id,
                  tool: approval.tool,
                  args: approval.args,
                  status: 'pending',
                  createdAt: approval.createdAt,
                  expiresAt: approval.expiresAt,
                }}
                onDecide={(approvalId, decision) =>
                  decideApproval(agent.id, approvalId, decision)
                }
              />
            ))
          )}
        </CardContent>
      </Card>

      <Card className="gap-3 lg:col-span-2">
        <CardHeader className="flex flex-row items-center justify-between gap-2">
          <CardTitle>{t('agentPage.overview.recent')}</CardTitle>
          <Button size="sm" variant="ghost" onClick={() => onOpenTab('runs')}>
            {t('agentPage.overview.allRuns')}
          </Button>
        </CardHeader>
        <CardContent>
          {recent.isPending ? (
            <Skeleton className="h-20" />
          ) : (recent.data?.rows.length ?? 0) === 0 ? (
            <div className="flex flex-wrap items-center gap-3 text-sm text-muted-foreground">
              <p>{t('agentPage.overview.noRuns')}</p>
              <Button
                size="sm"
                variant="outline"
                onClick={() => onOpenTab('chat')}
              >
                {t('agentPage.chatWith')}
              </Button>
            </div>
          ) : (
            <ul className="flex flex-col divide-y">
              {recent.data?.rows.map(run => (
                <AgentRunRow key={run.id} run={run} />
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <div className="lg:col-span-2">
        <TurnQueueStrip agentId={agent.id} agentName={agent.name} />
      </div>

      <Card className="gap-3 lg:col-span-2">
        <CardHeader>
          <CardTitle>{t('agentPage.overview.about')}</CardTitle>
        </CardHeader>
        <CardContent className="flex flex-col gap-3 text-sm">
          <p className="text-muted-foreground">
            {agent.status === 'error'
              ? t('agentPage.list.cantLoad')
              : agent.description || t('agentPage.list.noDescription')}
          </p>
          {workflows.length > 0 && (
            <p className="flex flex-wrap items-center gap-1">
              <span className="text-muted-foreground">
                {t('agentPage.overview.usedBy')}
              </span>
              {workflows.map(workflow => (
                <Button
                  key={workflow.id}
                  variant="link"
                  size="sm"
                  className="h-auto px-1"
                  onClick={() =>
                    useNavigationStore
                      .getState()
                      .navigate({ kind: 'workflow', workflowId: workflow.id })
                  }
                >
                  {workflow.name}
                </Button>
              ))}
            </p>
          )}
          <Button className="self-start" onClick={() => onOpenTab('chat')}>
            {t('agentPage.chatWith')}
          </Button>
        </CardContent>
      </Card>
    </div>
  )
}

/** The week in numbers, plus whatever stops this agent from running */
function HealthCard({
  agent,
  overview,
  container,
  onOpenTab,
}: {
  agent: AgentInfo
  overview: AgentOverview | undefined
  container: ContainerState
  onOpenTab: (tab: AgentTab) => void
}) {
  const { t } = useTranslation()
  const { data: defaultModel } = useDefaultModel()
  const harnesses = useHarnesses()
  const runtime = overview?.runtime ?? 'native'
  const completedShare =
    overview && overview.runs > 0
      ? Math.round((overview.completed / overview.runs) * 100)
      : null
  const badModel =
    agent.status === 'error' && hasModelFormatError(agent.description)
  const authMissing = runtimeAuthMissing(overview, harnesses.data)
  const noModel =
    !isCliRuntime(runtime) && defaultModel?.effectiveModel === null

  return (
    <Card className="gap-3">
      <CardHeader>
        <CardTitle>{t('agentPage.health.title')}</CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-2 text-sm">
        {!overview ? (
          <Skeleton className="h-12" />
        ) : (
          <>
            <p className="flex flex-wrap items-center gap-x-3 gap-y-1">
              <span>{t('agentPage.list.runs', { count: overview.runs })}</span>
              {completedShare !== null && (
                <span>
                  {t('agentPage.health.completed', { percent: completedShare })}
                </span>
              )}
              {overview.avgDurationMs !== null && (
                <span>
                  {t('agentPage.health.average', {
                    duration: formatDuration(overview.avgDurationMs),
                  })}
                </span>
              )}
              {overview.costUsd > 0 && (
                <span>
                  {t('agentPage.health.spent', {
                    cost: overview.costUsd.toFixed(2),
                  })}
                </span>
              )}
            </p>
            <AgentHealthCells days={overview.days} />
          </>
        )}
        <p className="text-muted-foreground">
          {t('agentPage.health.runtime', { runtime: runtimeName(runtime) })}
          {isCliRuntime(runtime) && ` · ${t('agentPage.where.container')}`}
          {container.status === 'error' &&
            ` · ${t('agentPage.container.error')}`}
        </p>
        {badModel && (
          <Fix
            text={t('agentPage.problem.badModel')}
            action={t('agentPage.health.pickModel')}
            onClick={() => onOpenTab('settings')}
          />
        )}
        {noModel && (
          <Fix
            text={t('agentPage.problem.noModel')}
            action={t('llmKeyNotice.action')}
            onClick={() => openSettings('models')}
          />
        )}
        {authMissing && (
          <Fix
            text={t('agentPage.problem.runtimeAuth')}
            action={t('agentPage.health.addKey')}
            onClick={() => openSettings('models')}
          />
        )}
      </CardContent>
    </Card>
  )
}

function Fix({
  text,
  action,
  onClick,
}: {
  text: string
  action: string
  onClick: () => void
}) {
  return (
    <div className="flex flex-wrap items-center gap-2 text-destructive">
      <TriangleAlert aria-hidden className="size-4 shrink-0" />
      <span>{text}</span>
      <Button size="sm" variant="outline" onClick={onClick}>
        {action}
      </Button>
    </div>
  )
}
