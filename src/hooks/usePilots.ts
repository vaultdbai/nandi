import { useEffect } from 'react'
import { useQueries, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { BrowserKind, PilotDefinition, PilotStatus } from '@/lib/bindings'
import { queryClient as appQueryClient } from '@/lib/query-client'
import { unwrapResult } from '@/lib/tauri-bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

/** Everything pilot-related lives under `['pilots']` */
export const pilotQueryKeys = {
  all: ['pilots'] as const,
  list: ['pilots', 'list'] as const,
  status: (pilotId: string) => ['pilots', 'status', pilotId] as const,
  permissions: (agentId: string) => ['pilots', 'permissions', agentId] as const,
  nandiBrowsers: ['pilots', 'nandiBrowsers'] as const,
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

  const list = useClientQuery(pilotQueryKeys.list, () => client.listPilots(), {
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
  return useClientMutation(
    ({ pilotId, action }: { pilotId: string; action: PilotAction }) => {
      switch (action) {
        case 'install':
          return client.installPilot(pilotId)
        case 'start':
          return client.startPilot(pilotId)
        case 'stop':
          return client.stopPilot(pilotId)
      }
    },
    {
      onSettled: (_data, _error, { pilotId }) =>
        queryClient.invalidateQueries({
          queryKey: pilotQueryKeys.status(pilotId),
        }),
    }
  )
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
  return useClientQuery(pilotQueryKeys.permissions(agentId), () =>
    client.agentPilotPermissions(agentId)
  )
}

/** Saves (or, empty, removes) a secret the agent's browser types */
export function useSetPilotSecret(agentId: string) {
  return useClientMutation(
    ({
      pilotId,
      name,
      value,
    }: {
      pilotId: string
      name: string
      value: string
    }) => client.setPilotSecret(agentId, pilotId, name, value),
    { invalidates: pilotQueryKeys.permissions(agentId) }
  )
}

/** Adds a site secret to the agent's browser (agent.md + keychain) */
export function useAddPilotSecret(agentId: string) {
  return useClientMutation(
    ({
      pilotId,
      name,
      value,
    }: {
      pilotId: string
      name: string
      value: string
    }) => client.addPilotSecret(agentId, pilotId, name, value),
    { invalidates: pilotQueryKeys.permissions(agentId) }
  )
}

/** Sign out everywhere: clears the agent's browser profile */
export function useClearBrowserProfile(agentId: string) {
  return useClientMutation(
    (pilotId: string) => client.clearBrowserProfile(agentId, pilotId),
    { invalidates: pilotQueryKeys.permissions(agentId) }
  )
}

/** Grants or revokes one capability of a pilot for an agent */
export function useSetPilotGrant(agentId: string) {
  return useClientMutation(
    ({
      pilotId,
      capability,
      granted,
    }: {
      pilotId: string
      capability: string
      granted: boolean
    }) =>
      granted
        ? client.grantPilotCapability(agentId, pilotId, capability)
        : client.revokePilotCapability(agentId, pilotId, capability),
    { invalidates: pilotQueryKeys.permissions(agentId) }
  )
}

/** My Nandi browser for each browser type installed here */
export function useNandiBrowsers() {
  return useClientQuery(pilotQueryKeys.nandiBrowsers, () =>
    client.nandiBrowsers()
  )
}

/** Opens the person's own window on My Nandi browser */
export function useOpenNandiBrowser() {
  return useClientMutation(
    (browser: BrowserKind) => client.openNandiBrowser(browser),
    { invalidates: pilotQueryKeys.nandiBrowsers }
  )
}

/** Sign out everywhere: wipes My Nandi browser's profile */
export function useSignOutNandiBrowser() {
  return useClientMutation(
    (browser: BrowserKind) => client.signOutNandiBrowser(browser),
    { invalidates: pilotQueryKeys.nandiBrowsers }
  )
}

/**
 * Lets the agent's visible browser use My Nandi browser, or (`null`) its
 * own profile again. Refreshes the agent's page and Settings › Browser.
 */
export function useSetAgentBrowserProfile(agentId: string) {
  return useClientMutation(
    (browser: BrowserKind | null) =>
      client.setAgentBrowserProfile(agentId, browser),
    { invalidates: pilotQueryKeys.all }
  )
}
