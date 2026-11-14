/**
 * The gateway agent: status, config, manifest (NANDI.md), callers, and the
 * provider-side queue. Kept live by `gateway://*` and `task://*` events
 * (`useGatewayEvents`).
 */
import { useEffect } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { CallerPolicy, GatewayManifest, TaskRecord } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

/** Everything gateway-related lives under `['gateway']` */
export const gatewayQueryKeys = {
  all: ['gateway'] as const,
  status: ['gateway', 'status'] as const,
  config: ['gateway', 'config'] as const,
  manifest: ['gateway', 'manifest'] as const,
  callers: ['gateway', 'callers'] as const,
  usage: (callerInstance: string) =>
    ['gateway', 'usage', callerInstance] as const,
  queue: ['gateway', 'queue'] as const,
}

/** Where an incoming request is, for Sharing › Incoming requests' filters */
export type IncomingFilter = 'waiting' | 'running' | 'done' | 'refused'

const INCOMING_FILTER: Record<TaskRecord['state'], IncomingFilter> = {
  submitted: 'waiting',
  delivered: 'waiting',
  acknowledged: 'running',
  working: 'running',
  inputRequired: 'running',
  completed: 'done',
  failed: 'done',
  cancelled: 'done',
  timedOut: 'done',
  abandoned: 'done',
  reassigned: 'done',
  rejected: 'refused',
}

export const incomingFilterOf = (state: TaskRecord['state']): IncomingFilter =>
  INCOMING_FILTER[state]

/** Newest first */
export function newestFirst(tasks: TaskRecord[]): TaskRecord[] {
  return [...tasks].sort((a, b) => b.createdAt - a.createdAt)
}

/** Comma- or newline-separated names, trimmed, distinct */
export function parseList(text: string): string[] {
  return [
    ...new Set(
      text
        .split(/[,\n]/)
        .map(item => item.trim())
        .filter(Boolean)
    ),
  ]
}

export function useGatewayStatus() {
  return useClientQuery(
    gatewayQueryKeys.status,
    () => client.getGatewayStatus(),
    { refetchInterval: 30_000 }
  )
}

export function useGatewayManifest() {
  return useClientQuery(gatewayQueryKeys.manifest, () =>
    client.getGatewayManifest()
  )
}

export function useGatewayCallers() {
  return useClientQuery(gatewayQueryKeys.callers, () =>
    client.listGatewayCallers()
  )
}

/** This month's use by one caller */
export function useGatewayUsage(callerInstance: string) {
  return useClientQuery(
    gatewayQueryKeys.usage(callerInstance),
    () => client.getGatewayUsage(callerInstance),
    { enabled: callerInstance.length > 0 }
  )
}

/**
 * Every task this instance provided or refused (refused ones included:
 * they used to vanish). Fails quietly while the task lifecycle is
 * unavailable (the view shows nothing yet).
 */
export function useIncomingTasks() {
  return useQuery({
    queryKey: gatewayQueryKeys.queue,
    queryFn: async () =>
      newestFirst(
        unwrapResult(
          await client.listTasks({
            state: null,
            role: 'provider',
            agentId: null,
          })
        )
      ),
    retry: false,
    refetchInterval: 15_000,
  })
}

/**
 * Turns on `relay.enabled` and `relay.discoverable` and re-registers the
 * card; resolves to the status (`registered`: the relay took it)
 */
export function useMakeGatewayReachable() {
  const queryClient = useQueryClient()
  return useClientMutation(() => client.makeGatewayReachable(), {
    onSuccess: () =>
      queryClient.invalidateQueries({ queryKey: gatewayQueryKeys.all }),
  })
}

/** Saves all of NANDI.md: frontmatter and body */
export function useSetGatewayManifest() {
  const queryClient = useQueryClient()
  return useClientMutation(
    (manifest: GatewayManifest) => client.setGatewayManifest(manifest),
    {
      onSuccess: saved => {
        queryClient.setQueryData(gatewayQueryKeys.manifest, saved)
        void queryClient.invalidateQueries({
          queryKey: gatewayQueryKeys.status,
        })
        void queryClient.invalidateQueries({
          queryKey: gatewayQueryKeys.config,
        })
      },
    }
  )
}

export function useSetCallerPolicy() {
  const queryClient = useQueryClient()
  return useClientMutation(
    (policy: CallerPolicy) => client.setCallerPolicy(policy),
    {
      onSuccess: () =>
        queryClient.invalidateQueries({ queryKey: gatewayQueryKeys.callers }),
    }
  )
}

export function useRemoveCallerPolicy() {
  const queryClient = useQueryClient()
  return useClientMutation(
    (callerInstance: string) => client.removeCallerPolicy(callerInstance),
    {
      onSuccess: () =>
        queryClient.invalidateQueries({ queryKey: gatewayQueryKeys.callers }),
    }
  )
}

/** Refreshes the gateway queries when the core reports a change */
export function useGatewayEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const refresh = (...keys: (readonly string[])[]) => {
      for (const queryKey of keys) {
        void queryClient.invalidateQueries({ queryKey })
      }
    }
    const subscription = client.subscribe(event => {
      switch (event.event) {
        case 'gateway://config_changed':
          refresh(
            gatewayQueryKeys.manifest,
            gatewayQueryKeys.config,
            gatewayQueryKeys.status
          )
          break
        case 'gateway://caller_updated':
          refresh(gatewayQueryKeys.callers)
          break
        case 'gateway://rate_limited':
        case 'task://state_changed':
          refresh(gatewayQueryKeys.status, gatewayQueryKeys.queue)
          break
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])
}

/**
 * One message to the gateway agent, answered as a caller would see it.
 * Nothing is cached: the conversation lives in the chat panel.
 */
export function useGatewayChat() {
  return useClientMutation((message: string) => client.gatewayChat(message))
}
