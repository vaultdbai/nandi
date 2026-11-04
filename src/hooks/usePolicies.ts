import { useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'

export const policiesQueryKeys = {
  all: ['policies'] as const,
}

/**
 * Every policy file in `.nandi/policies/`, sorted by id: its name, and
 * whether it parses (a broken one still opens, to be fixed)
 */
export function usePolicies() {
  return useQuery({
    queryKey: policiesQueryKeys.all,
    queryFn: async () => unwrapResult(await client.listPolicies()),
  })
}
