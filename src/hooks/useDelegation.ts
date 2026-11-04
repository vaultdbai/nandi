/**
 * Providers, delegation history, and per-agent delegation settings
 * (TanStack Query), kept live by `delegation://` events.
 *
 * Card IDs are `<instance_id>/<agent_id>`. The user's own requests come
 * from the card `<instance_id>/_user`.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type {
  AgentCard,
  AgentDelegationSettings,
  JsonValue,
  TaskResult,
  TaskStatus,
  TrustRule,
} from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

/** Rows the history view loads */
export const HISTORY_LIMIT = 500

/** Everything delegation-related lives under `['delegation']` */
export const delegationQueryKeys = {
  all: ['delegation'] as const,
  history: ['delegation', 'history'] as const,
  providersAll: ['delegation', 'providers'] as const,
  providers: (capability: string) =>
    ['delegation', 'providers', capability] as const,
  settings: (agentId: string) => ['delegation', 'settings', agentId] as const,
  trust: ['delegation', 'trust'] as const,
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/** The agent part of a card ID */
export function agentIdOf(cardId: string): string {
  const slash = cardId.indexOf('/')
  return slash === -1 ? cardId : cardId.slice(slash + 1)
}

/** The other side of a row: who we called, or who called us */
export function counterpart(row: TaskResult): {
  cardId: string
  owner: string | null
} {
  return row.direction === 'outbound'
    ? { cardId: row.providerAgent, owner: row.providerOwner }
    : { cardId: row.requesterAgent, owner: row.requesterOwner }
}

export interface HistoryFilters {
  status?: TaskStatus | ''
  capability?: string
  owner?: string
}

export function filterHistory(
  rows: TaskResult[],
  filters: HistoryFilters
): TaskResult[] {
  return rows.filter(row => {
    if (filters.status && row.status !== filters.status) return false
    if (filters.capability && row.capability !== filters.capability)
      return false
    if (filters.owner && counterpart(row).owner !== filters.owner) return false
    return true
  })
}

/** Distinct capabilities and owner handles, sorted, for the filter menus */
export function historyFacets(rows: TaskResult[]): {
  capabilities: string[]
  owners: string[]
} {
  const capabilities = new Set<string>()
  const owners = new Set<string>()
  for (const row of rows) {
    capabilities.add(row.capability)
    const { owner } = counterpart(row)
    if (owner) owners.add(owner)
  }
  return {
    capabilities: [...capabilities].sort(),
    owners: [...owners].sort(),
  }
}

/** Provider cards by capability (sorted), without the agent's own card */
export function providersByCapability(
  cards: AgentCard[],
  selfAgentId?: string
): [string, AgentCard[]][] {
  const byCapability = new Map<string, AgentCard[]>()
  for (const card of cards) {
    if (selfAgentId && agentIdOf(card.id) === selfAgentId) continue
    for (const capability of card.capabilities) {
      const list = byCapability.get(capability.name) ?? []
      list.push(card)
      byCapability.set(capability.name, list)
    }
  }
  return [...byCapability.entries()].sort(([a], [b]) => a.localeCompare(b))
}

/** The card for a local agent in a provider list (local cards come first) */
export function ownCard(
  cards: AgentCard[],
  agentId: string
): AgentCard | undefined {
  return cards.find(card => agentIdOf(card.id) === agentId)
}

/**
 * Who may call `agentId`: requesters with an `allow` trust rule (`*` is
 * anyone), plus those it has served before. Sorted, distinct.
 */
export function callersOf(
  agentId: string,
  rules: TrustRule[],
  history: TaskResult[]
): string[] {
  const callers = new Set<string>()
  for (const rule of rules) {
    if (rule.providerAgent === agentId && rule.policy === 'allow') {
      callers.add(rule.requester)
    }
  }
  for (const row of history) {
    if (
      row.direction === 'inbound' &&
      agentIdOf(row.providerAgent) === agentId &&
      row.status === 'completed'
    ) {
      callers.add(row.requesterAgent)
    }
  }
  return [...callers].sort()
}

/** A readable message from whatever a query or mutation threw */
export function errorMessage(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  if (
    error !== null &&
    typeof error === 'object' &&
    'error' in error &&
    typeof error.error === 'string'
  ) {
    return error.error
  }
  return String(error)
}

/** Puts a changed row first, replacing its older copy */
export function upsertRow(rows: TaskResult[], row: TaskResult): TaskResult[] {
  return [row, ...rows.filter(r => r.taskId !== row.taskId)]
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/** Keeps delegation queries current from `delegation://` events */
export function useDelegationEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      switch (event.event) {
        case 'delegation://task_completed': {
          const { result } = event.payload
          queryClient.setQueryData<TaskResult[]>(
            delegationQueryKeys.history,
            rows => (rows ? upsertRow(rows, result) : rows)
          )
          void queryClient.invalidateQueries({
            queryKey: delegationQueryKeys.providersAll,
          })
          return
        }
        case 'delegation://task_started':
        case 'delegation://task_failed':
          void queryClient.invalidateQueries({
            queryKey: delegationQueryKeys.history,
          })
          void queryClient.invalidateQueries({
            queryKey: delegationQueryKeys.providersAll,
          })
          return
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])
}

/** Delegation history, newest first, both directions, kept live */
export function useDelegation(limit = HISTORY_LIMIT) {
  useDelegationEvents()
  const history = useQuery({
    queryKey: delegationQueryKeys.history,
    queryFn: async () =>
      unwrapResult(await client.listDelegationHistory(limit)),
  })
  return {
    history: history.data ?? [],
    isLoading: history.isLoading,
    isFetching: history.isFetching,
    error: history.error,
    refetch: history.refetch,
  }
}

/** Providers serving `capability`; `''` lists every provider */
export function useAvailableProviders(capability = '') {
  return useQuery({
    queryKey: delegationQueryKeys.providers(capability),
    queryFn: async () =>
      unwrapResult(await client.listAvailableProviders(capability)),
  })
}

/** Trust rules (empty while they can't be read) */
export function useTrustRules() {
  return useQuery({
    queryKey: delegationQueryKeys.trust,
    queryFn: async () => {
      const result = await client.listTrustPolicies()
      return result.status === 'ok' ? result.data : []
    },
  })
}

export function useAgentDelegation(agentId: string) {
  return useQuery({
    queryKey: delegationQueryKeys.settings(agentId),
    queryFn: async () => unwrapResult(await client.getAgentDelegation(agentId)),
  })
}

/** Saves the settings into skill.md; optimistic, rolled back on error */
export function useSetAgentDelegation(agentId: string) {
  const queryClient = useQueryClient()
  const key = delegationQueryKeys.settings(agentId)
  return useMutation({
    mutationFn: async (settings: AgentDelegationSettings) =>
      unwrapResult(await client.setAgentDelegation(agentId, settings)),
    onMutate: async settings => {
      await queryClient.cancelQueries({ queryKey: key })
      const previous = queryClient.getQueryData<AgentDelegationSettings>(key)
      queryClient.setQueryData(key, settings)
      return { previous }
    },
    onError: (_error, _settings, context) => {
      if (context?.previous) queryClient.setQueryData(key, context.previous)
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: key })
      void queryClient.invalidateQueries({
        queryKey: delegationQueryKeys.providersAll,
      })
    },
  })
}

/** Starts a delegation as the user; resolves to the task ID */
export function useDelegateTask() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      capability,
      input,
      target,
    }: {
      capability: string
      input: JsonValue
      target?: string | null
    }) =>
      unwrapResult(
        await client.delegateTask(capability, input, target ?? null)
      ),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: delegationQueryKeys.history }),
  })
}
