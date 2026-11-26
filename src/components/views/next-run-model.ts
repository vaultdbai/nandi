/**
 * Automations' Next run: when a workflow or a trigger runs next, from the
 * scheduled triggers' next times (`list_next_runs`, soonest first). Pure.
 */
import type { NextRun, TriggerTargetType } from '@/lib/bindings'

/** The soonest scheduled run that starts this agent or workflow */
export function nextRunOfTarget(
  runs: readonly NextRun[],
  targetType: TriggerTargetType,
  targetId: string
): number | null {
  const times = runs
    .filter(run => run.targetType === targetType && run.targetId === targetId)
    .map(run => run.at)
  return times.length > 0 ? Math.min(...times) : null
}

/** When this trigger runs next; `null` when it waits for something else */
export function nextRunOfTrigger(
  runs: readonly NextRun[],
  triggerId: string
): number | null {
  return runs.find(run => run.triggerId === triggerId)?.at ?? null
}
