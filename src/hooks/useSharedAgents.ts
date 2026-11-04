/**
 * Agents shared with me and by me, trust policies, and the data the trust
 * graph draws from. Kept live by relay presence events here, and by the
 * `relay://`, `team://`, and `delegation://` invalidations in agent-sync.
 */

import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type {
  CapabilityProviders,
  JsonValue,
  ProviderOption,
  ShareSource,
  SharedAgent,
  TrustPolicy,
} from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

/** Everything this track caches lives under `['shared']` */
export const sharedQueryKeys = {
  all: ['shared'] as const,
  withMe: ['shared', 'withMe'] as const,
  byMe: ['shared', 'byMe'] as const,
  policies: ['shared', 'trustPolicies'] as const,
  /** The trust graph's copy of the delegation history */
  history: ['shared', 'delegationHistory'] as const,
  /** The trust graph's copy of my teams */
  teams: ['shared', 'teams'] as const,
  capabilitiesAll: ['shared', 'capabilities'] as const,
  /** Who serves each capability (`null`: every capability) */
  capabilities: (capability: string | null) =>
    ['shared', 'capabilities', capability ?? '*'] as const,
}

/** How many history rows the trust graph reads (newest first) */
export const TRUST_GRAPH_HISTORY_LIMIT = 500

export type SourceFilter = 'all' | ShareSource

export interface SharedAgentFilter {
  source: SourceFilter
  search: string
}

/**
 * Agents matching `filter`: from the chosen source, and whose name,
 * description, owner, team, or a capability contains the search text.
 */
export function filterSharedAgents(
  agents: SharedAgent[],
  filter: SharedAgentFilter
): SharedAgent[] {
  const needle = filter.search.trim().toLowerCase()
  return agents.filter(agent => {
    if (filter.source !== 'all' && !agent.sources.includes(filter.source)) {
      return false
    }
    if (!needle) return true
    const { card } = agent
    const haystack = [
      card.name,
      card.description,
      card.owner.githubHandle,
      card.owner.displayName,
      ...card.capabilities.map(c => c.name),
      ...agent.teamIds,
    ]
    return haystack.some(text => text.toLowerCase().includes(needle))
  })
}

/** "Recently used": called within the last week */
export const RECENTLY_USED_MS = 7 * 24 * 60 * 60 * 1000

/** Agents called within the last week */
export function recentlyUsed(
  agents: SharedAgent[],
  now: number = Date.now()
): SharedAgent[] {
  return agents.filter(
    agent =>
      agent.lastCalledAt !== null &&
      now - agent.lastCalledAt <= RECENTLY_USED_MS
  )
}

/** One owner's agents, for the grouped shared grids */
export interface OwnerGroup {
  /** The handle (lowercased), else the instance ID */
  key: string
  handle: string
  displayName: string
  /** Where their agents run (the first card's), for "View profile" */
  instanceId: string | null
  agents: SharedAgent[]
}

/**
 * Groups agents by owner handle (case-insensitively; cards without one
 * group by instance), keeping each group's agent order. Groups sort by
 * name, then key.
 */
export function groupByOwner(agents: SharedAgent[]): OwnerGroup[] {
  const groups = new Map<string, OwnerGroup>()
  for (const agent of agents) {
    const { owner, instanceId } = agent.card
    const handle = owner.githubHandle.trim()
    const key = handle ? handle.toLowerCase() : instanceId
    let group = groups.get(key)
    if (!group) {
      group = {
        key,
        handle,
        displayName: owner.displayName.trim(),
        instanceId: instanceId || null,
        agents: [],
      }
      groups.set(key, group)
    }
    group.agents.push(agent)
  }
  return [...groups.values()].sort((a, b) => {
    const name = (group: OwnerGroup) =>
      (group.displayName || group.handle || group.key).toLowerCase()
    return name(a).localeCompare(name(b)) || a.key.localeCompare(b.key)
  })
}

/**
 * Input for a quick call: JSON when it parses, else the raw text (an empty
 * box sends `null`).
 */
export function parseCallInput(text: string): JsonValue {
  const trimmed = text.trim()
  if (!trimmed) return null
  try {
    return JSON.parse(trimmed) as JsonValue
  } catch {
    return trimmed
  }
}

/** How many agents serve each capability */
export function providerCounts(
  overview: CapabilityProviders[] | undefined
): Map<string, number> {
  return new Map(
    (overview ?? []).map(entry => [entry.capability, entry.providers.length])
  )
}

/** The provider an auto-picked task would go to first */
export function autoProvider(
  entry: CapabilityProviders | undefined
): ProviderOption | undefined {
  if (!entry?.autoChoice) return undefined
  return entry.providers.find(p => p.card.id === entry.autoChoice)
}

/** The agent's name with its owner, for the provider pickers */
export function providerLabel(provider: ProviderOption): string {
  const owner = provider.card.owner.githubHandle
  return owner ? `${provider.card.name} (@${owner})` : provider.card.name
}

/**
 * Capabilities whose name, or one of whose providers' name, owner, or
 * team, contains the search text.
 */
export function filterCapabilities(
  overview: CapabilityProviders[],
  search: string
): CapabilityProviders[] {
  const needle = search.trim().toLowerCase()
  if (!needle) return overview
  return overview.filter(entry => {
    if (entry.capability.toLowerCase().includes(needle)) return true
    return entry.providers.some(({ card, teamIds }) =>
      [
        card.name,
        card.owner.githubHandle,
        card.owner.displayName,
        ...teamIds,
      ].some(text => text.toLowerCase().includes(needle))
    )
  })
}

export function useSharedWithMe() {
  return useQuery({
    queryKey: sharedQueryKeys.withMe,
    queryFn: async () => unwrapResult(await client.listAgentsSharedWithMe()),
  })
}

export function useSharedByMe() {
  return useQuery({
    queryKey: sharedQueryKeys.byMe,
    queryFn: async () => unwrapResult(await client.listAgentsSharedByMe()),
  })
}

export function useTrustPolicies() {
  return useQuery({
    queryKey: sharedQueryKeys.policies,
    queryFn: async () => unwrapResult(await client.listTrustPolicies()),
  })
}

/** Per capability: who serves it, ranked as auto would pick, and my pin */
export function useCapabilityProviders(capability: string | null = null) {
  return useQuery({
    queryKey: sharedQueryKeys.capabilities(capability),
    queryFn: async () =>
      unwrapResult(await client.listCapabilityProviders(capability)),
    enabled: capability !== '',
  })
}

/** Pins a provider for a capability (my settings), or unpins it */
export function useSetProviderPin() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (pin: { capability: string; cardId: string | null }) =>
      unwrapResult(await client.setProviderPin(pin.capability, pin.cardId)),
    onSettled: () =>
      queryClient.invalidateQueries({
        queryKey: sharedQueryKeys.capabilitiesAll,
      }),
  })
}

/** Delegation history for the trust graph */
export function useTrustGraphHistory() {
  return useQuery({
    queryKey: sharedQueryKeys.history,
    queryFn: async () =>
      unwrapResult(
        await client.listDelegationHistory(TRUST_GRAPH_HISTORY_LIMIT)
      ),
    retry: false,
  })
}

/** My teams for the trust graph */
export function useTrustGraphTeams() {
  return useQuery({
    queryKey: sharedQueryKeys.teams,
    queryFn: async () => unwrapResult(await client.listTeams()),
    retry: false,
  })
}

/** Both shared lists, refetched as agents come and go at the relay. */
export function useSharedAgents() {
  const queryClient = useQueryClient()
  const withMe = useSharedWithMe()
  const byMe = useSharedByMe()

  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (
        event.event === 'relay://agent_online' ||
        event.event === 'relay://agent_offline'
      ) {
        void queryClient.invalidateQueries({
          queryKey: sharedQueryKeys.withMe,
        })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return { withMe, byMe }
}

export function useSetTrustPolicy() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (rule: {
      requester: string
      providerAgent: string
      policy: TrustPolicy
    }) =>
      unwrapResult(
        await client.setTrustPolicy(
          rule.requester,
          rule.providerAgent,
          rule.policy
        )
      ),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: sharedQueryKeys.policies }),
  })
}

/**
 * Starts a delegation; resolves to the task ID. `cardId: null` lets Nandi
 * pick (pin, then ranking, with fallback).
 */
export function useCallSharedAgent() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (call: {
      cardId: string | null
      capability: string
      input: JsonValue
    }) =>
      unwrapResult(
        await client.delegateTask(call.capability, call.input, call.cardId)
      ),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: sharedQueryKeys.all }),
  })
}

export type StopSharing =
  | { kind: 'team'; teamId: string; agentId: string }
  | { kind: 'relay'; agentId: string }

/**
 * Stops one way an agent is shared: removes it from a team, or turns off
 * `accepts_delegation` (keeping the agent's other delegation settings).
 */
export function useStopSharing() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (stop: StopSharing) => {
      if (stop.kind === 'team') {
        unwrapResult(await client.unshareAgent(stop.teamId, stop.agentId))
        return
      }
      const settings = unwrapResult(
        await client.getAgentDelegation(stop.agentId)
      )
      unwrapResult(
        await client.setAgentDelegation(stop.agentId, {
          ...settings,
          acceptInbound: false,
        })
      )
    },
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: sharedQueryKeys.all }),
  })
}
