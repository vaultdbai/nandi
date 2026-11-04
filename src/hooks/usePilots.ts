import { useEffect } from 'react'
import {
  useMutation,
  useQueries,
  useQuery,
  useQueryClient,
} from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { PilotDefinition, PilotStatus } from '@/lib/bindings'
import { queryClient as appQueryClient } from '@/lib/query-client'
import { unwrapResult } from '@/lib/tauri-bindings'

/** Everything pilot-related lives under `['pilots']` */
export const pilotQueryKeys = {
  all: ['pilots'] as const,
  list: ['pilots', 'list'] as const,
  status: (pilotId: string) => ['pilots', 'status', pilotId] as const,
  permissions: (agentId: string) => ['pilots', 'permissions', agentId] as const,
}

/** `windows` | `macos` | `linux`, as `PilotDefinition.platform` spells it */
export function currentPlatform(): string {
  const agent = typeof navigator === 'undefined' ? '' : navigator.userAgent
  if (/Windows/i.test(agent)) return 'windows'
  if (/Mac OS X|Macintosh/i.test(agent)) return 'macos'
  return 'linux'
}

/** Whether a pilot runs on `platform` (its own, or a cross-platform one) */
export function runsOn(pilot: PilotDefinition, platform: string): boolean {
  return pilot.platform === platform || pilot.platform === 'any'
}

/**
 * Pilot list and status, kept live by `pilot://` events. The core already
 * filters the list to the host OS; `runsOn` keeps the UI honest in a
 * browser on another machine.
 */
export function usePilots() {
  const queryClient = useQueryClient()

  const list = useQuery({
    queryKey: pilotQueryKeys.list,
    queryFn: async () => unwrapResult(await client.listPilots()),
    staleTime: Infinity,
  })
  const pilots = list.data ?? []

  const statusQueries = useQueries({
    queries: pilots.map(pilot => ({
      queryKey: pilotQueryKeys.status(pilot.id),
      queryFn: async () => unwrapResult(await client.pilotStatus(pilot.id)),
    })),
  })
  const statuses: Record<string, PilotStatus | undefined> = {}
  pilots.forEach((pilot, index) => {
    statuses[pilot.id] = statusQueries[index]?.data
  })

  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (event.event === 'pilot://changed') {
        queryClient.setQueryData(
          pilotQueryKeys.status(event.payload.pilotId),
          event.payload.status
        )
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return {
    pilots,
    statuses,
    isLoading: list.isLoading,
    error: list.error,
  }
}

type PilotAction = 'install' | 'start' | 'stop'

/** Install / start / stop. Status arrives as `pilot://changed` too. */
export function usePilotAction() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      pilotId,
      action,
    }: {
      pilotId: string
      action: PilotAction
    }) => {
      switch (action) {
        case 'install':
          unwrapResult(await client.installPilot(pilotId))
          return
        case 'start':
          unwrapResult(await client.startPilot(pilotId))
          return
        case 'stop':
          unwrapResult(await client.stopPilot(pilotId))
          return
      }
    },
    onSettled: (_data, _error, { pilotId }) =>
      queryClient.invalidateQueries({
        queryKey: pilotQueryKeys.status(pilotId),
      }),
  })
}

/** Grants an agent each listed capability of a pilot, in order. */
export async function grantPilotCapabilities(
  agentId: string,
  pilotId: string,
  capabilities: string[]
): Promise<string | null> {
  try {
    for (const capability of capabilities) {
      const result = await client.grantPilotCapability(
        agentId,
        pilotId,
        capability
      )
      if (result.status === 'error') return result.error
    }
    return null
  } finally {
    void appQueryClient.invalidateQueries({
      queryKey: pilotQueryKeys.permissions(agentId),
    })
  }
}

/** The pilots an agent declares, and which capabilities it was granted */
export function useAgentPilotPermissions(agentId: string) {
  return useQuery({
    queryKey: pilotQueryKeys.permissions(agentId),
    queryFn: async () =>
      unwrapResult(await client.agentPilotPermissions(agentId)),
  })
}

/** Grants or revokes one capability of a pilot for an agent */
/** Saves (or, empty, removes) a secret the agent's browser types */
export function useSetPilotSecret(agentId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      pilotId,
      name,
      value,
    }: {
      pilotId: string
      name: string
      value: string
    }) =>
      unwrapResult(await client.setPilotSecret(agentId, pilotId, name, value)),
    onSettled: () =>
      queryClient.invalidateQueries({
        queryKey: pilotQueryKeys.permissions(agentId),
      }),
  })
}

export function useSetPilotGrant(agentId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      pilotId,
      capability,
      granted,
    }: {
      pilotId: string
      capability: string
      granted: boolean
    }) => {
      const result = granted
        ? await client.grantPilotCapability(agentId, pilotId, capability)
        : await client.revokePilotCapability(agentId, pilotId, capability)
      unwrapResult(result)
    },
    onSettled: () =>
      queryClient.invalidateQueries({
        queryKey: pilotQueryKeys.permissions(agentId),
      }),
  })
}
