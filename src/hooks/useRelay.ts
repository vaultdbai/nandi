import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import { inviteLine } from '@/components/network/relay-invite'
import { client } from '@/lib/client'
import type { AgentCard, JsonValue, RelayStatus } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

/** Everything relay-related lives under `['relay']` */
export const relayQueryKeys = {
  all: ['relay'] as const,
  status: ['relay', 'status'] as const,
  instances: ['relay', 'instances'] as const,
  agents: (capability: string | null) =>
    ['relay', 'agents', capability] as const,
  /** The public-relay account of the relay in use (the Account card) */
  publicAccount: ['relay', 'publicAccount'] as const,
}

/** "Join the public relay": asked only when someone opens the flow */
export const PUBLIC_RELAY_OFFER_KEY = ['publicRelay', 'offer'] as const

/**
 * Whether the public relay can be joined now (its `/health`, via the
 * core). Never retried on its own: unreachable is an answer, and "Try
 * again" refetches.
 */
export function usePublicRelayOffer(enabled = true) {
  return useQuery({
    queryKey: PUBLIC_RELAY_OFFER_KEY,
    queryFn: async () => unwrapResult(await client.getPublicRelayOffer()),
    enabled,
    retry: false,
    staleTime: 30_000,
  })
}

/** The public-relay account this Nandi is signed in with */
export function usePublicRelayAccount(enabled: boolean) {
  return useQuery({
    queryKey: relayQueryKeys.publicAccount,
    queryFn: async () => unwrapResult(await client.getPublicRelayAccount()),
    enabled,
    retry: false,
    // Today's usage moves as people send
    refetchInterval: enabled ? 60_000 : false,
  })
}

/** Cards grouped by the capabilities they serve, capabilities sorted */
export function groupByCapability(
  cards: AgentCard[]
): { capability: string; cards: AgentCard[] }[] {
  const groups = new Map<string, AgentCard[]>()
  for (const card of cards) {
    for (const { name } of card.capabilities) {
      const group = groups.get(name) ?? []
      if (!group.some(c => c.id === card.id)) group.push(card)
      groups.set(name, group)
    }
  }
  return [...groups.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([capability, group]) => ({
      capability,
      cards: [...group].sort((a, b) => a.name.localeCompare(b.name)),
    }))
}

/** Cards whose name, description, owner, or capabilities match `search` */
export function filterAgents(cards: AgentCard[], search: string): AgentCard[] {
  const needle = search.trim().toLowerCase()
  if (!needle) return cards
  return cards.filter(card =>
    [
      card.name,
      card.description,
      card.owner.githubHandle,
      card.owner.displayName,
      ...card.capabilities.map(c => c.name),
    ].some(text => text.toLowerCase().includes(needle))
  )
}

/**
 * Relay status and the agents online, kept live by `relay://status`,
 * `relay://agent_online`, and `relay://agent_offline`.
 */
export function useRelay() {
  const queryClient = useQueryClient()

  const status = useQuery({
    queryKey: relayQueryKeys.status,
    queryFn: async () => unwrapResult(await client.getRelayStatus()),
  })
  const agents = useQuery({
    queryKey: relayQueryKeys.agents(null),
    queryFn: async () => unwrapResult(await client.listOnlineAgents(null)),
  })

  useEffect(() => {
    const subscription = client.subscribe(event => {
      switch (event.event) {
        case 'relay://status': {
          const previous = queryClient.getQueryData<RelayStatus>(
            relayQueryKeys.status
          )
          queryClient.setQueryData(relayQueryKeys.status, event.payload)
          // Connecting brings a fresh list
          if (event.payload.connected && !previous?.connected) {
            void queryClient.invalidateQueries({
              queryKey: ['relay', 'agents'],
            })
          }
          break
        }
        case 'relay://agent_online': {
          const { card } = event.payload
          queryClient.setQueryData<AgentCard[]>(
            relayQueryKeys.agents(null),
            old => [...(old ?? []).filter(c => c.id !== card.id), card]
          )
          break
        }
        case 'relay://agent_offline': {
          const { cardId } = event.payload
          queryClient.setQueryData<AgentCard[]>(
            relayQueryKeys.agents(null),
            old => (old ?? []).filter(c => c.id !== cardId)
          )
          break
        }
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return {
    status: status.data,
    agents: agents.data ?? [],
    isLoading: status.isLoading || agents.isLoading,
    error: status.error ?? agents.error,
  }
}

/**
 * Whether the relay is connected right now (`undefined` until known).
 * Person-to-person places show `RelayConnectNotice` when it's `false`.
 */
export function useRelayConnected(): boolean | undefined {
  return useRelay().status?.connected
}

/** `GET /trust/relay`: "Trust everyone on this relay" and whether it applies */
export const RELAY_TRUST_KEY = ['trust', 'relay'] as const

export function useRelayTrust() {
  return useQuery({
    queryKey: RELAY_TRUST_KEY,
    queryFn: async () => unwrapResult(await client.getRelayTrust()),
  })
}

/** This Nandi's instance ID (`null` until the status is read) */
export function useOwnInstanceId(): string | null {
  const status = useQuery({
    queryKey: relayQueryKeys.status,
    queryFn: async () => unwrapResult(await client.getRelayStatus()),
  })
  return status.data?.instanceId ?? null
}

/** Saves the relay URL (empty: no relay); the core reconnects */
export function useSetRelayUrl() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (url: string) =>
      unwrapResult(await client.setRelayUrl(url.trim())),
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: relayQueryKeys.all }),
  })
}

/**
 * "Join": joins a relay (enrolling with the join code when it is secured).
 * Resolves with the outcome; only a local failure rejects. Refreshes the
 * relay queries once joined.
 */
export function useJoinRelay() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      url,
      joinCode,
    }: {
      url: string
      joinCode?: string | null
    }) =>
      unwrapResult(
        await client.joinRelay(url.trim(), joinCode?.trim() || null)
      ),
    onSuccess: outcome => {
      if (outcome.kind === 'joined')
        void queryClient.invalidateQueries({ queryKey: relayQueryKeys.all })
    },
  })
}

/**
 * "Copy invite message": asks the core for the invite (the relay without
 * this Nandi's token, and the join code the user typed for it, if any)
 * only when clicked, and copies the message without showing the code
 */
export function useCopyInvite() {
  const { t } = useTranslation()
  return async () => {
    const result = await client.getRelayInvite()
    if (result.status === 'error' || !result.data) {
      toast.error(t('network.invite.failed'))
      return
    }
    const invite = result.data
    const message = t(
      invite.joinCode
        ? 'network.invite.messageWithCode'
        : 'network.invite.message',
      { invite: inviteLine(invite) }
    )
    try {
      await navigator.clipboard.writeText(message)
      toast.success(t('network.copied'))
    } catch {
      toast.error(t('network.invite.failed'))
    }
  }
}

/**
 * The instances at the relay (its `/instances` endpoint), refreshed every
 * 10s while connected. Fails when the relay is unreachable or hides the
 * list (a relay with a secret); the view then counts the instances it
 * learned from presence.
 */
export function useRelayInstances(connected: boolean) {
  return useQuery({
    queryKey: relayQueryKeys.instances,
    queryFn: async () => unwrapResult(await client.getRelayInstances()),
    enabled: connected,
    refetchInterval: connected ? 10_000 : false,
    retry: false,
  })
}

/** "Test connection": the round trip in ms, or the exact error */
export function useTestRelayConnection() {
  return useMutation({
    mutationFn: async (url: string) =>
      unwrapResult(await client.testRelayConnection(url.trim())),
  })
}

/** "Call": delegates `input` to one online agent; returns the task ID */
export function useCallAgent() {
  return useMutation({
    mutationFn: async ({
      card,
      capability,
      input,
    }: {
      card: AgentCard
      capability: string
      input: JsonValue
    }) => unwrapResult(await client.delegateTask(capability, input, card.id)),
  })
}
