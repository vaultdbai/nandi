import type { WorkflowRunStatus } from '@/lib/bindings'
import type { RunStatus, RunSummary } from '@/lib/tauri-bindings'

/**
 * Badge styles per run status, shared by the dashboard and run views.
 * Green = done, blue = active, amber = waiting on you.
 */
export const RUN_STATUS_CLASSES: Record<RunStatus, string> = {
  queued: 'border-border bg-muted text-muted-foreground',
  running: 'border-info/30 bg-info/10 text-info',
  awaitingApproval: 'border-warning/30 bg-warning/10 text-warning',
  completed: 'border-success/30 bg-success/10 text-success',
  failed: 'border-destructive/30 bg-destructive/10 text-destructive',
  cancelled: 'border-border bg-muted text-muted-foreground',
  compacted: 'border-highlight/30 bg-highlight/10 text-highlight',
}

/** Badge styles per workflow run status (paused waits on you) */
export const WORKFLOW_RUN_STATUS_CLASSES: Record<WorkflowRunStatus, string> = {
  running: RUN_STATUS_CLASSES.running,
  awaitingApproval: RUN_STATUS_CLASSES.awaitingApproval,
  completed: RUN_STATUS_CLASSES.completed,
  failed: RUN_STATUS_CLASSES.failed,
  paused: RUN_STATUS_CLASSES.awaitingApproval,
  waiting: RUN_STATUS_CLASSES.awaitingApproval,
  cancelled: RUN_STATUS_CLASSES.cancelled,
}

/** A run and the runs that continued it after compaction */
export interface RunTreeNode {
  run: RunSummary
  children: RunTreeNode[]
}

/**
 * Groups runs into compaction trees: each child (oldest first) under its
 * `parentRunId`. Runs whose parent isn't in `runs` are roots, in the
 * order given.
 */
export function groupRunTree(runs: RunSummary[]): RunTreeNode[] {
  const nodes = new Map<string, RunTreeNode>()
  for (const run of runs) nodes.set(run.id, { run, children: [] })

  const roots: RunTreeNode[] = []
  for (const node of nodes.values()) {
    const { parentRunId } = node.run
    const parent =
      parentRunId && parentRunId !== node.run.id
        ? nodes.get(parentRunId)
        : undefined
    if (parent) parent.children.push(node)
    else roots.push(node)
  }
  for (const node of nodes.values()) {
    node.children.sort((a, b) => a.run.startedAt - b.run.startedAt)
  }
  return roots
}

function contains(node: RunTreeNode, runId: string): boolean {
  return (
    node.run.id === runId || node.children.some(child => contains(child, runId))
  )
}

/** The compaction tree `runId` belongs to, if it's in `runs` */
export function runLineage(
  runs: RunSummary[],
  runId: string
): RunTreeNode | null {
  return groupRunTree(runs).find(root => contains(root, runId)) ?? null
}
