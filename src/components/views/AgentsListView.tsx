import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { Bot, Plus, Search, TriangleAlert } from 'lucide-react'
import { AgentHealthCells } from '@/components/agents/AgentHealthCells'
import { DeleteAgentDialog } from '@/components/agents/DeleteAgentDialog'
import { NewAgentDialog } from '@/components/agents/NewAgentDialog'
import {
  agentProblem,
  runtimeAuthMissing,
  runtimeDetail,
  runtimeName,
  sharedLabel,
  sortAgents,
  type AgentProblem,
} from '@/components/agents/agent-page-model'
import { AgentMenu } from '@/components/sharing/AgentMenu'
import { AgentSharingDialog } from '@/components/sharing/AgentSharingDialog'
import { AgentAvatar } from '@/components/agent-console/AgentAvatar'
import { AgentStatusPill } from '@/components/agent-console/AgentStatusIndicator'
import { effectiveStatus } from '@/components/agent-console/agent-status'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Empty,
  EmptyContent,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from '@/components/ui/empty'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import { ScrollArea } from '@/components/ui/scroll-area'
import { Skeleton } from '@/components/ui/skeleton'
import { useAgentOverviews, useAgents } from '@/hooks/useAgents'
import { sharingOf } from '@/hooks/useAgentSharing'
import { useHarnesses } from '@/hooks/useHarnesses'
import { useDefaultModel } from '@/hooks/useLlmKeyStatus'
import { useSharedByMe } from '@/hooks/useSharedAgents'
import { formatRelativeTime } from '@/lib/format-time'
import type { AgentOverview } from '@/lib/bindings'
import { cn } from '@/lib/utils'
import { useAgentStore, useCurrentRun } from '@/store/agent-store'
import { useNavigationStore, type View } from '@/store/navigation-store'
import type { AgentInfo } from '@/types/agent'
import { ViewHeader } from './ViewHeader'

const navigate = (view: View) => useNavigationStore.getState().navigate(view)

interface Row {
  agent: AgentInfo
  name: string
  overview: AgentOverview | undefined
  problem: AgentProblem | null
  lastRunAt: number | null
}

/**
 * Every agent on this Nandi: what it does, where it runs, how its week went
 * and whether others can use it. Agents waiting on me come first, then
 * ones with a problem. A row opens the agent; its "⋯" is always visible.
 */
export function AgentsListView() {
  const { t } = useTranslation()
  const agents = useAgents()
  const overviews = useAgentOverviews()
  const harnesses = useHarnesses()
  const { data: defaultModel } = useDefaultModel()
  const pendingByAgent = useAgentStore(state => state.agents)
  const [search, setSearch] = useState('')
  const [runtime, setRuntime] = useState('all')
  const [acceptsOnly, setAcceptsOnly] = useState(false)
  const [creating, setCreating] = useState(false)

  const noModel = defaultModel?.effectiveModel === null
  const rows: Row[] = (agents.data ?? []).map(agent => {
    const overview = overviews.data?.find(o => o.agentId === agent.id)
    return {
      agent,
      name: agent.name,
      builtin: agent.builtin,
      overview,
      problem: agentProblem({
        agent,
        pending: pendingByAgent[agent.id]?.pendingApprovals.length ?? 0,
        noModel: noModel && (overview?.runtime ?? 'native') === 'native',
        runtimeAuthMissing: runtimeAuthMissing(overview, harnesses.data),
      }),
      lastRunAt: overview?.lastRunAt ?? null,
    }
  })
  const query = search.trim().toLowerCase()
  const shown = sortAgents(rows).filter(
    row =>
      (!query ||
        row.agent.name.toLowerCase().includes(query) ||
        row.agent.description.toLowerCase().includes(query)) &&
      (runtime === 'all' || (row.overview?.runtime ?? 'native') === runtime) &&
      (!acceptsOnly || row.overview?.acceptsRequests === true)
  )
  const runtimes = [
    ...new Set(rows.map(row => row.overview?.runtime ?? 'native')),
  ]

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ViewHeader
        title={t('nav.agents')}
        subtitle={t('agentPage.list.subtitle')}
        actions={
          <>
            <Button
              size="sm"
              variant="outline"
              onClick={() => navigate({ kind: 'library' })}
            >
              {t('agentPage.list.browseLibrary')}
            </Button>
            <Button size="sm" onClick={() => setCreating(true)}>
              <Plus />
              {t('agentPage.list.newAgent')}
            </Button>
          </>
        }
      />
      <NewAgentDialog open={creating} onOpenChange={setCreating} />
      {rows.length > 0 && (
        <div className="flex flex-wrap items-center gap-3 border-b px-6 py-3">
          <div className="relative min-w-48 flex-1">
            <Search
              aria-hidden
              className="pointer-events-none absolute start-2.5 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
            />
            <Input
              value={search}
              onChange={event => setSearch(event.target.value)}
              placeholder={t('agentPage.list.search')}
              aria-label={t('agentPage.list.search')}
              className="ps-8"
            />
          </div>
          <NativeSelect
            aria-label={t('agentPage.list.runtimeFilter')}
            value={runtime}
            onChange={event => setRuntime(event.target.value)}
          >
            <NativeSelectOption value="all">
              {t('agentPage.list.allRuntimes')}
            </NativeSelectOption>
            {runtimes.map(id => (
              <NativeSelectOption key={id} value={id}>
                {runtimeName(id)}
              </NativeSelectOption>
            ))}
          </NativeSelect>
          <div className="flex items-center gap-2">
            <Checkbox
              id="agents-accepts-only"
              checked={acceptsOnly}
              onCheckedChange={checked => setAcceptsOnly(checked === true)}
            />
            <Label htmlFor="agents-accepts-only" className="text-sm">
              {t('agentPage.list.acceptsOnly')}
            </Label>
          </div>
        </div>
      )}
      <ScrollArea className="min-h-0 flex-1">
        {agents.isLoading ? (
          <div className="mx-auto flex w-full max-w-6xl flex-col gap-2 p-6">
            {[0, 1, 2, 3, 4].map(i => (
              <Skeleton key={i} className="h-16 rounded-lg" />
            ))}
          </div>
        ) : agents.error ? (
          <Empty className="py-16">
            <EmptyHeader>
              <EmptyTitle>{t('agentPage.list.loadError')}</EmptyTitle>
            </EmptyHeader>
            <EmptyContent>
              <Button variant="outline" onClick={() => void agents.refetch()}>
                {t('agentPage.list.tryAgain')}
              </Button>
            </EmptyContent>
          </Empty>
        ) : rows.length === 0 ? (
          <Empty className="py-16">
            <EmptyHeader>
              <EmptyMedia variant="icon">
                <Bot />
              </EmptyMedia>
              <EmptyTitle>{t('agentConsole.agents.empty')}</EmptyTitle>
              <EmptyDescription>
                {t('agentPage.list.emptyDescription')}
              </EmptyDescription>
            </EmptyHeader>
            <EmptyContent className="flex-row flex-wrap justify-center">
              <Button onClick={() => navigate({ kind: 'starter-packs' })}>
                {t('agentPage.list.installPack')}
              </Button>
              <Button
                variant="link"
                onClick={() =>
                  navigate({ kind: 'studio', conversationId: null })
                }
              >
                {t('agentPage.list.describeInStudio')}
              </Button>
            </EmptyContent>
          </Empty>
        ) : shown.length === 0 ? (
          <Empty className="py-16">
            <EmptyHeader>
              <EmptyTitle>{t('agentPage.list.noMatch')}</EmptyTitle>
            </EmptyHeader>
            <EmptyContent>
              <Button
                variant="outline"
                onClick={() => {
                  setSearch('')
                  setRuntime('all')
                  setAcceptsOnly(false)
                }}
              >
                {t('agentPage.list.clearFilters')}
              </Button>
            </EmptyContent>
          </Empty>
        ) : (
          <ul
            aria-label={t('nav.agents')}
            className="mx-auto flex w-full max-w-6xl flex-col divide-y p-6 pt-3"
          >
            {shown.map(row => (
              <AgentRow
                key={row.agent.id}
                row={row}
                defaultModel={
                  defaultModel?.effectiveModel ??
                  defaultModel?.defaultModel ??
                  null
                }
              />
            ))}
          </ul>
        )}
      </ScrollArea>
    </div>
  )
}

function AgentRow({
  row,
  defaultModel,
}: {
  row: Row
  defaultModel: string | null
}) {
  const { t, i18n } = useTranslation()
  const { agent, overview, problem } = row
  const run = useCurrentRun(agent.id)
  const byMe = useSharedByMe()
  const [menuOpen, setMenuOpen] = useState(false)
  const [sharingOpen, setSharingOpen] = useState(false)
  const [deleting, setDeleting] = useState(false)

  const detail = overview ? runtimeDetail(overview, defaultModel) : null
  const sharing = sharingOf(byMe.data, agent.id)
  const shared = sharedLabel(agent)
  const sharedText =
    shared.kind === 'from'
      ? t('agentPage.list.from', { publisher: shared.publisher })
      : sharing.teamIds.length > 0 && sharing.relay
        ? t('agentPage.list.teamAndRelay')
        : sharing.teamIds.length > 0
          ? t('agentPage.list.team')
          : sharing.relay
            ? t('agentPage.list.relay')
            : '—'
  const open = (tab?: 'settings') =>
    navigate({ kind: 'agent', agentId: agent.id, ...(tab && { tab }) })

  return (
    <li
      className="flex items-center gap-2 py-1"
      onContextMenu={event => {
        event.preventDefault()
        setMenuOpen(true)
      }}
    >
      <button
        type="button"
        onClick={() => open()}
        className="grid min-w-0 flex-1 grid-cols-1 items-center gap-x-4 gap-y-1 rounded-md px-2 py-2 text-start hover:bg-accent/40 focus-visible:ring-[3px] focus-visible:ring-ring/50 focus-visible:outline-none md:grid-cols-[minmax(0,2fr)_minmax(0,1fr)_minmax(0,1.6fr)_6rem_5rem]"
      >
        <span className="flex min-w-0 items-center gap-3">
          <AgentAvatar agent={agent} className="size-8 shrink-0" />
          <span className="min-w-0">
            <span className="flex items-center gap-2">
              <span className="truncate font-medium">{agent.name}</span>
              <AgentStatusPill status={effectiveStatus(agent, run)} />
            </span>
            <span className="block truncate text-xs text-muted-foreground">
              {agent.status === 'error'
                ? t('agentPage.list.cantLoad')
                : agent.description || t('agentPage.list.noDescription')}
            </span>
          </span>
        </span>
        <span className="min-w-0 text-sm">
          <span className="block truncate">
            {runtimeName(overview?.runtime ?? 'native')}
          </span>
          <span className="block truncate text-xs text-muted-foreground">
            {detail?.kind === 'container'
              ? t('agentPage.where.container')
              : detail?.label || t('agentPage.model.default')}
          </span>
        </span>
        <span className="min-w-0 text-sm">
          <span className="flex items-center gap-2">
            {overview && <AgentHealthCells days={overview.days} />}
            <span className="truncate text-xs text-muted-foreground">
              {!overview || overview.runs === 0
                ? t('agentPage.list.noRuns')
                : overview.failed > 0
                  ? t('agentPage.list.runsFailed', {
                      count: overview.runs,
                      failed: overview.failed,
                    })
                  : t('agentPage.list.runs', { count: overview.runs })}
            </span>
          </span>
          {problem ? (
            <ProblemLine problem={problem} />
          ) : (
            overview?.lastRunAt != null && (
              <span className="block truncate text-xs text-muted-foreground">
                {t('agentPage.list.lastRun', {
                  when: formatRelativeTime(overview.lastRunAt, i18n.language),
                })}
              </span>
            )
          )}
        </span>
        <span className="truncate text-sm text-muted-foreground">
          {sharedText}
        </span>
        <span className="text-sm">
          {overview?.acceptsRequests
            ? t('agentPage.list.on')
            : t('agentPage.list.off')}
        </span>
      </button>
      <AgentMenu
        agent={agent}
        open={menuOpen}
        onOpenChange={setMenuOpen}
        onOpenSharing={() => setSharingOpen(true)}
        onDelete={() => setDeleting(true)}
      />
      <AgentSharingDialog
        agent={agent}
        open={sharingOpen}
        onOpenChange={setSharingOpen}
      />
      <DeleteAgentDialog
        agent={agent}
        open={deleting}
        onOpenChange={setDeleting}
      />
    </li>
  )
}

/** One problem, as a sentence that names its fix */
function ProblemLine({ problem }: { problem: AgentProblem }) {
  const { t } = useTranslation()
  const text =
    problem.kind === 'waiting'
      ? t('agentPage.problem.waiting', { count: problem.count })
      : t(`agentPage.problem.${problem.kind}`)
  return (
    <span
      className={cn(
        'flex items-center gap-1 truncate text-xs',
        problem.kind === 'waiting' ? 'text-warning' : 'text-destructive'
      )}
    >
      <TriangleAlert aria-hidden className="size-3 shrink-0" />
      {text}
    </span>
  )
}
