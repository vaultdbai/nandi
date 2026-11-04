import { useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { Badge } from '@/components/ui/badge'
import { useRunDetail } from '@/hooks/useSnapshots'
import { runsQueryKeys } from '@/hooks/useRuns'
import { client } from '@/lib/client'
import {
  RUN_STATUS_CLASSES,
  runLineage,
  type RunTreeNode,
} from '@/lib/run-status'
import { unwrapResult } from '@/lib/tauri-bindings'
import { cn } from '@/lib/utils'
import { useNavigationStore } from '@/store/navigation-store'
import { RunDetailView } from './RunDetailView'

/** Enough of an agent's history to hold a compaction chain */
const LINEAGE_RUN_LIMIT = 200

/**
 * `{ kind: 'run' }` links (dashboard, workflows, triggers, All runs) open
 * the tabbed run detail view; the agent ID in the link isn't needed. A run
 * that's part of a compaction chain also shows the chain as a tree, each
 * child run indented under the run it continues.
 */
export function RunHistoryView({ runId }: { runId: string }) {
  return (
    <div className="flex h-full min-h-0 flex-col">
      <RunLineage runId={runId} />
      <div className="min-h-0 flex-1">
        <RunDetailView runId={runId} />
      </div>
    </div>
  )
}

function useAgentRuns(agentId: string | undefined) {
  return useQuery({
    queryKey: [...runsQueryKeys.all, 'lineage', agentId],
    queryFn: async () =>
      unwrapResult(await client.listAllRuns({ agentId }, LINEAGE_RUN_LIMIT, 0))
        .rows,
    enabled: agentId !== undefined,
  })
}

function RunLineage({ runId }: { runId: string }) {
  const { t } = useTranslation()
  const { data: detail } = useRunDetail(runId)
  const { data: runs } = useAgentRuns(detail?.run.agentId)

  const tree = runs ? runLineage(runs, runId) : null
  if (!tree || tree.children.length === 0) return null

  return (
    <nav
      aria-label={t('runs.compactionLineage')}
      className="shrink-0 border-b px-4 py-2"
    >
      <p className="mb-1 text-xs font-medium text-muted-foreground">
        {t('runs.compactionLineage')}
      </p>
      <ul className="flex flex-col gap-0.5">
        <LineageNode node={tree} depth={0} currentId={runId} />
      </ul>
    </nav>
  )
}

function LineageNode({
  node,
  depth,
  currentId,
}: {
  node: RunTreeNode
  depth: number
  currentId: string
}) {
  const { t } = useTranslation()
  const { run } = node
  const current = run.id === currentId

  return (
    <li>
      <button
        type="button"
        aria-current={current ? 'page' : undefined}
        style={{ paddingInlineStart: `${depth * 1.25}rem` }}
        className={cn(
          'flex w-full items-center gap-2 rounded-sm py-0.5 text-start text-xs hover:bg-muted',
          current && 'font-medium'
        )}
        onClick={() =>
          useNavigationStore
            .getState()
            .navigate({ kind: 'run-detail', runId: run.id })
        }
      >
        {depth > 0 && (
          <span aria-hidden className="text-muted-foreground">
            ↳
          </span>
        )}
        <code dir="ltr" className="font-mono" title={run.id}>
          {run.id.length > 12 ? run.id.slice(0, 8) : run.id}
        </code>
        <Badge
          variant="outline"
          className={cn('text-[10px]', RUN_STATUS_CLASSES[run.status])}
        >
          {t(`runs.status.${run.status}`)}
        </Badge>
      </button>
      {node.children.length > 0 && (
        <ul className="flex flex-col gap-0.5">
          {node.children.map(child => (
            <LineageNode
              key={child.run.id}
              node={child}
              depth={depth + 1}
              currentId={currentId}
            />
          ))}
        </ul>
      )}
    </li>
  )
}
