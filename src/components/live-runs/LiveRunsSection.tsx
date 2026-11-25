import { useTranslation } from 'react-i18next'
import { AgentAvatar } from '@/components/agent-console/AgentAvatar'
import { useNow } from '@/components/agents/task-utils'
import { useAgents } from '@/hooks/useAgents'
import { useRunPlan } from '@/hooks/useRunPlan'
import { useLiveRuns } from '@/hooks/useRuns'
import { displayAgent } from '@/lib/agent-identity'
import type { AgentInfo, RunSummary } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import {
  labelText,
  liveGroup,
  liveOrigin,
  orderLive,
  statusLine,
  statusText,
} from './live-run-model'
import { LiveBadge } from './LiveRunView'

/**
 * Ask's Live section: what needs the person first, then what's working,
 * then what finished in the last hour (faded), each opening as a chat.
 * No live work: no section at all.
 */
export function LiveRunsSection({ className }: { className?: string }) {
  const { t } = useTranslation()
  const now = useNow(true, 60_000)
  const runs = orderLive(useLiveRuns().data ?? [], now)
  const agents = useAgents().data ?? []
  if (runs.length === 0) return null

  return (
    <section aria-labelledby="live-runs-heading" className={className}>
      <h2
        id="live-runs-heading"
        className="px-1 pb-1 text-xs font-medium text-muted-foreground"
      >
        {t('liveWork.heading')}
      </h2>
      <ul aria-label={t('liveWork.heading')} className="flex flex-col gap-1">
        {runs.map(run => (
          <li key={run.id}>
            <LiveRow run={run} agents={agents} />
          </li>
        ))}
      </ul>
    </section>
  )
}

/** One live chat: the agent, what it's doing now, and whether it needs you */
function LiveRow({
  run,
  agents,
}: {
  run: RunSummary
  agents: readonly AgentInfo[]
}) {
  const { t, i18n } = useTranslation()
  const group = liveGroup(run)
  const { data: plan } = useRunPlan(group === 'finished' ? null : run.id)
  const agentOf = (id: string) => agents.find(a => a.id === id)
  const agent = agentOf(run.agentId) ?? displayAgent(run.agentId, run.agentId)
  const line = statusLine(plan, [])
  const origin = labelText(
    liveOrigin(run, id => agentOf(id)?.name),
    t
  )

  return (
    <button
      type="button"
      data-live-group={group}
      onClick={() =>
        useNavigationStore
          .getState()
          .navigate({ kind: 'live-run', runId: run.id })
      }
      className={cn(
        'flex w-full items-center gap-3 rounded-md border px-3 py-2 text-start hover:bg-accent',
        group === 'needsYou' && 'border-warning/40 bg-warning/5',
        group === 'finished' && 'opacity-60'
      )}
    >
      <AgentAvatar agent={agent} className="size-6 shrink-0" />
      <span className="flex min-w-0 flex-1 flex-col">
        <span className="truncate text-sm font-medium">{agent.name}</span>
        <span className="truncate text-xs text-muted-foreground">
          {line ? `${statusText(line, t)} · ${origin}` : origin}
        </span>
      </span>
      <LiveBadge status={run.status} />
      <span className="shrink-0 text-xs text-muted-foreground">
        {formatRelativeTime(run.endedAt ?? run.startedAt, i18n.language)}
      </span>
    </button>
  )
}
