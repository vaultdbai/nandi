/**
 * The connector hub (`docs/developer/connector-hub.md`): grants, connectors
 * from the registries, and the read-only calendars and drives. Grants
 * refetch on `grants://changed`, accounts on `workspace://changed`, and
 * registry connectors on `connectors://changed`.
 */
import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { connectorsQueryKeys } from '@/hooks/useConnectors'
import { useSignInFlow } from '@/hooks/useEmail'
import { client } from '@/lib/client'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'
import type { Result } from '@/lib/tauri-bindings'
import type {
  AccountKind,
  GrantAccess,
  GrantInfo,
  GrantScope,
  PublishConnectorInput,
  ScaffoldConnectorInput,
  WorkspaceStatus,
} from '@/lib/bindings'

export const hubKeys = {
  grants: ['connector-grants'] as const,
  workspace: ['workspace', 'status'] as const,
  registry: ['connectors', 'registry'] as const,
}

const EVENT_KEYS: Record<string, readonly (readonly string[])[]> = {
  'grants://changed': [hubKeys.grants],
  'workspace://changed': [hubKeys.workspace, hubKeys.grants],
  'connectors://changed': [hubKeys.registry],
}

function useHubEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      for (const queryKey of EVENT_KEYS[event.event] ?? []) {
        void queryClient.invalidateQueries({ queryKey })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])
}

/** Every grant, saved and this computer's */
export function useConnectorGrants() {
  useHubEvents()
  return useClientQuery(hubKeys.grants, () => client.listConnectorGrants())
}

export type GrantChange =
  | {
      kind: 'grant'
      agentId: string
      connectorId: string
      access: GrantAccess
      scope: GrantScope
    }
  | { kind: 'revoke'; grant: GrantInfo }

export function useChangeGrant() {
  return useClientMutation(
    (change: GrantChange) =>
      change.kind === 'grant'
        ? client.grantConnectorAccess(
            change.agentId,
            change.connectorId,
            change.access,
            change.scope
          )
        : client.revokeConnectorAccess(change.grant),
    { invalidates: hubKeys.grants }
  )
}

/** Calendars and drives, and the calendar check */
export function useWorkspaceStatus() {
  useHubEvents()
  return useClientQuery(hubKeys.workspace, () => client.getWorkspaceStatus())
}

export function useWorkspaceAction() {
  const queryClient = useQueryClient()
  return useClientMutation(
    (
      action:
        | { kind: 'check' }
        | { kind: 'interval'; minutes: number }
        | { kind: 'disconnect'; account: AccountKind }
    ): Promise<Result<WorkspaceStatus, string>> => {
      switch (action.kind) {
        case 'check':
          return client.checkCalendarNow()
        case 'interval':
          return client.setCalendarInterval(action.minutes)
        case 'disconnect':
          return client.disconnectWorkspace(action.account)
      }
    },
    {
      onSuccess: status => {
        queryClient.setQueryData(hubKeys.workspace, status)
        void queryClient.invalidateQueries({ queryKey: hubKeys.grants })
      },
    }
  )
}

export function useWorkspaceSignIn(account: AccountKind) {
  return useSignInFlow(
    account,
    () => client.startWorkspaceSignIn(account),
    () => client.pollWorkspaceSignIn(account),
    hubKeys.workspace
  )
}

/** Connectors listed in the configured registries */
export function useRegistryConnectors() {
  useHubEvents()
  return useClientQuery(hubKeys.registry, () =>
    client.listRegistryConnectors(false)
  )
}

/** What installing one would mean, for the consent dialog */
export function useConnectorConsent(
  target: { registryUrl: string; connectorId: string } | null
) {
  return useClientQuery(
    [
      ...hubKeys.registry,
      'consent',
      target?.registryUrl ?? '',
      target?.connectorId ?? '',
    ],
    () =>
      client.getConnectorConsent(
        target?.registryUrl ?? '',
        target?.connectorId ?? ''
      ),
    { enabled: target !== null }
  )
}

export function useInstallRegistryConnector() {
  return useClientMutation(
    (input: {
      registryUrl: string
      connectorId: string
      apiKey: string | null
    }) =>
      client.installRegistryConnector(
        input.registryUrl,
        input.connectorId,
        input.apiKey
      ),
    { invalidates: connectorsQueryKeys.all }
  )
}

export function useRemoveConnector() {
  return useClientMutation((id: string) => client.removeConnector(id), {
    invalidates: connectorsQueryKeys.all,
  })
}

export function useScaffoldConnector() {
  return useClientMutation(
    (input: ScaffoldConnectorInput) => client.scaffoldConnector(input),
    { invalidates: connectorsQueryKeys.all }
  )
}

export function usePublishConnector() {
  return useClientMutation(
    (input: PublishConnectorInput) => client.publishConnector(input),
    { invalidates: connectorsQueryKeys.all }
  )
}
