/**
 * A run's step-by-step plan (`plan_set` / `plan_update`) and its
 * screenshots. `run://plan_updated` writes the plan straight into the cache
 * (`lib/agent-sync.ts`), so the checklist card updates live, never polls.
 */
import { client } from '@/lib/client'
import { useClientQuery } from '@/hooks/useClientQuery'

export const runPlanQueryKeys = {
  plan: (runId: string) => ['runPlan', runId] as const,
  screenshot: (runId: string, path: string) =>
    ['runScreenshot', runId, path] as const,
}

export function useRunPlan(runId: string | null) {
  return useClientQuery(
    runPlanQueryKeys.plan(runId ?? ''),
    () => client.getRunPlan(runId ?? ''),
    {
      enabled: runId !== null,
      staleTime: Infinity,
    }
  )
}

/** A screenshot as a `data:` URL (a file doesn't change once taken) */
export function useRunScreenshot(runId: string, path: string) {
  return useClientQuery(
    runPlanQueryKeys.screenshot(runId, path),
    () => client.getRunScreenshot(runId, path),
    { staleTime: Infinity }
  )
}
