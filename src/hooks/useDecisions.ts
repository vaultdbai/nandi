/**
 * The Decisions page's data: the months that have decisions, and one
 * month's entries (`list_decisions`). Local files only, no relay. A
 * decision made elsewhere in the app shows up on the next refetch.
 */
import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { client, type NandiEventName } from '@/lib/client'
import { useInvalidateOn } from '@/hooks/useInvalidateOn'
import type { DecisionFilter } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

export const decisionKeys = {
  all: ['decisions'] as const,
  months: ['decisions', 'months'] as const,
  list: (month: string, filter: DecisionFilter | null) =>
    ['decisions', 'list', month, filter] as const,
}

/** Events after which a decision may have been written */
const DECISION_EVENTS: readonly NandiEventName[] = [
  'agent://approval_resolved',
  'gateway://hold_changed',
  'gateway://assignment_changed',
  'workflow://manual_step_completed',
  'request://updated',
  'request://returned',
]

export function useDecisionMonths() {
  useInvalidateOn(DECISION_EVENTS, decisionKeys.all)
  return useQuery({
    queryKey: decisionKeys.months,
    queryFn: async () => unwrapResult(await client.listDecisionMonths()),
  })
}

export function useDecisions(
  month: string | null,
  filter: DecisionFilter | null
) {
  return useQuery({
    queryKey: decisionKeys.list(month ?? '', filter),
    queryFn: async () =>
      unwrapResult(await client.listDecisions(month ?? '', filter)),
    enabled: month !== null,
    placeholderData: keepPreviousData,
  })
}
