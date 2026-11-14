import { client } from '@/lib/client'
import { useClientQuery } from '@/hooks/useClientQuery'

export const policiesQueryKeys = {
  all: ['policies'] as const,
}

/**
 * Every policy file in `.nandi/policies/`, sorted by id: its name, and
 * whether it parses (a broken one still opens, to be fixed)
 */
export function usePolicies() {
  return useClientQuery(policiesQueryKeys.all, () => client.listPolicies())
}
