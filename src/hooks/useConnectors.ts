/**
 * Connector list and status, kept live by `connector://` events.
 */
import { useEffect } from 'react'
import { useQueryClient, type QueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { AgentConnectorChoice, JsonValue } from '@/lib/bindings'
import type {
  ConnectorInfo,
  ConnectorStatus,
  Result,
} from '@/lib/tauri-bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

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
  return useClientQuery(
    connectorsQueryKeys.list,
    () => client.listConnectors() as Promise<Result<ConnectorDetails[], string>>
  )
}

export type ConnectorAction = 'install' | 'start' | 'stop'

/** Install, start, or stop; the list refreshes from the events */
export function useConnectorAction() {
  return useClientMutation(
    ({ id, action }: { id: string; action: ConnectorAction }) => {
      switch (action) {
        case 'install':
          return client.installConnector(id)
        case 'start':
          return client.startConnector(id)
        case 'stop':
          return client.stopConnector(id)
      }
    },
    { invalidates: connectorsQueryKeys.all }
  )
}

/** Saves a secret (an empty value clears it); the value never comes back */
export function useSetConnectorSecret() {
  return useClientMutation(
    ({ id, key, value }: { id: string; key: string; value: string }) =>
      client.setConnectorSecret(id, key, value),
    { invalidates: connectorsQueryKeys.all }
  )
}

/** Replaces an installed connector's settings (written to the gold copy) */
export function useConfigureConnector() {
  return useClientMutation(
    ({ id, config }: { id: string; config: Record<string, JsonValue> }) =>
      client.configureConnector(id, config),
    { invalidates: connectorsQueryKeys.all }
  )
}

/** Which agents get each of a filesystem connector's shared folders */
export function useFolderUsers(connectorId: string) {
  return useClientQuery(
    [...connectorsQueryKeys.all, 'folder-users', connectorId],
    () => client.listFolderUsers(connectorId)
  )
}

/** The connectors an agent's agent.md declares, with its folders */
export function useAgentConnectors(agentId: string) {
  return useClientQuery(connectorsQueryKeys.agent(agentId), () =>
    client.getAgentConnectors(agentId)
  )
}

/** Replaces them (writes the agent's agent.md) */
export function useSetAgentConnectors(agentId: string) {
  return useClientMutation(
    (connectors: AgentConnectorChoice[]) =>
      client.setAgentConnectors(agentId, connectors),
    // The agent's own list, and who uses each folder
    { invalidates: connectorsQueryKeys.all }
  )
}
