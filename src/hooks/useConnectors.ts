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
import type {
  ConnectorInfo,
  ConnectorStatus,
  Result,
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

async function unwrap<T>(result: Promise<Result<T, string>>): Promise<T> {
  const resolved = await result
  if (resolved.status === 'error') throw new Error(resolved.error)
  return resolved.data
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
      } else if (
        event.event === 'connectors://changed' ||
        event.event === 'nandi://resync'
      ) {
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
      (await unwrap(client.listConnectors())) as ConnectorDetails[],
  })
}

/** Live status (health-checked by the core) for one connector */
export function useConnectorStatus(id: string | null) {
  return useQuery({
    queryKey: connectorsQueryKeys.status(id ?? ''),
    queryFn: () => unwrap(client.connectorStatus(id ?? '')),
    enabled: id !== null,
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
          return unwrap(client.installConnector(id))
        case 'start':
          return unwrap(client.startConnector(id))
        case 'stop':
          return unwrap(client.stopConnector(id))
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
    }) => unwrap(client.setConnectorSecret(id, key, value)),
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
    }) => unwrap(client.configureConnector(id, config)),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: connectorsQueryKeys.all }),
  })
}

/** Which agents get each of a filesystem connector's shared folders */
export function useFolderUsers(connectorId: string) {
  return useQuery({
    queryKey: [...connectorsQueryKeys.all, 'folder-users', connectorId],
    queryFn: () => unwrap(client.listFolderUsers(connectorId)),
  })
}

/** The connectors an agent's skill.md declares, with its folders */
export function useAgentConnectors(agentId: string) {
  return useQuery({
    queryKey: connectorsQueryKeys.agent(agentId),
    queryFn: () => unwrap(client.getAgentConnectors(agentId)),
  })
}

/** Replaces them (writes the agent's skill.md) */
export function useSetAgentConnectors(agentId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (connectors: AgentConnectorChoice[]) =>
      unwrap(client.setAgentConnectors(agentId, connectors)),
    onSettled: () =>
      // The agent's own list, and who uses each folder
      queryClient.invalidateQueries({ queryKey: connectorsQueryKeys.all }),
  })
}
