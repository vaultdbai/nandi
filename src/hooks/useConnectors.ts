/**
 * Connector list and status, kept live by `connector://` events.
 */
import { useEffect } from 'react'
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { AgentConnectorChoice, JsonValue } from '@/lib/bindings'
import {
  unwrapResult,
  type ConnectorInfo,
  type ConnectorStatus,
} from '@/lib/tauri-bindings'

/**
 * Fields the backend adds to `ConnectorInfo`/`ConnectorStatus` that the
 * generated bindings don't have yet (regenerate with `pnpm rust:bindings`).
 * Optional so the UI works with either.
 */
export interface ConnectorTool {
  name: string
  description: string
  sensitive: boolean
}

export interface ConnectorSecret {
  key: string
  keyringEntry: string
  isSet: boolean
}

export type ConnectorDetails = ConnectorInfo & {
  status: ConnectorStatus & { error?: string | null }
  tools?: ConnectorTool[]
  config?: Record<string, unknown>
  secrets?: ConnectorSecret[]
  setupHint?: string | null
}

export const connectorsQueryKeys = {
  all: ['connectors'] as const,
  list: ['connectors', 'list'] as const,
  status: (id: string) => ['connectors', 'status', id] as const,
  agent: (agentId: string) => ['connectors', 'agent', agentId] as const,
}

/** Applies a `connector://changed` status to the cached list in place */
function applyStatus(
  queryClient: QueryClient,
  connectorId: string,
  status: ConnectorStatus
) {
  queryClient.setQueryData<ConnectorDetails[]>(
    connectorsQueryKeys.list,
    current =>
      current?.map(connector =>
        connector.id === connectorId ? { ...connector, status } : connector
      )
  )
  queryClient.setQueryData(connectorsQueryKeys.status(connectorId), status)
}

/** Keeps the connector queries in step with the core's events */
function useConnectorEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (event.event === 'connector://changed') {
        applyStatus(
          queryClient,
          event.payload.connectorId,
          event.payload.status
        )
        // Tools and sign-in state come with the list
        void queryClient.invalidateQueries({
          queryKey: connectorsQueryKeys.list,
        })
      } else if (event.event === 'connectors://changed') {
        void queryClient.invalidateQueries({
          queryKey: connectorsQueryKeys.all,
        })
      }
    })
    return subscription.unsubscribe
  }, [queryClient])
}

/** The catalog plus everything declared in `.nandi/connectors.yaml` */
export function useConnectors() {
  useConnectorEvents()
  return useQuery({
    queryKey: connectorsQueryKeys.list,
    queryFn: async () =>
      (await client.listConnectors().then(unwrapResult)) as ConnectorDetails[],
  })
}

export type ConnectorAction = 'install' | 'start' | 'stop'

/** Install, start, or stop; the list refreshes from the events */
export function useConnectorAction() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, action }: { id: string; action: ConnectorAction }) => {
      switch (action) {
        case 'install':
          return client.installConnector(id).then(unwrapResult)
        case 'start':
          return client.startConnector(id).then(unwrapResult)
        case 'stop':
          return client.stopConnector(id).then(unwrapResult)
      }
    },
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: connectorsQueryKeys.all }),
  })
}

/** Saves a secret (an empty value clears it); the value never comes back */
export function useSetConnectorSecret() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({
      id,
      key,
      value,
    }: {
      id: string
      key: string
      value: string
    }) => client.setConnectorSecret(id, key, value).then(unwrapResult),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: connectorsQueryKeys.all }),
  })
}

/** Replaces an installed connector's settings (written to the gold copy) */
export function useConfigureConnector() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({
      id,
      config,
    }: {
      id: string
      config: Record<string, JsonValue>
    }) => client.configureConnector(id, config).then(unwrapResult),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: connectorsQueryKeys.all }),
  })
}

/** Which agents get each of a filesystem connector's shared folders */
export function useFolderUsers(connectorId: string) {
  return useQuery({
    queryKey: [...connectorsQueryKeys.all, 'folder-users', connectorId],
    queryFn: () => client.listFolderUsers(connectorId).then(unwrapResult),
  })
}

/** The connectors an agent's skill.md declares, with its folders */
export function useAgentConnectors(agentId: string) {
  return useQuery({
    queryKey: connectorsQueryKeys.agent(agentId),
    queryFn: () => client.getAgentConnectors(agentId).then(unwrapResult),
  })
}

/** Replaces them (writes the agent's skill.md) */
export function useSetAgentConnectors(agentId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (connectors: AgentConnectorChoice[]) =>
      client.setAgentConnectors(agentId, connectors).then(unwrapResult),
    onSettled: () =>
      // The agent's own list, and who uses each folder
      queryClient.invalidateQueries({ queryKey: connectorsQueryKeys.all }),
  })
}
