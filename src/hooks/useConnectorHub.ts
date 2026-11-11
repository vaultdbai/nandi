/**
 * The connector hub (`docs/developer/connector-hub.md`): grants, connectors
 * from the registries, and the read-only calendars and drives. Grants
 * refetch on `grants://changed`, accounts on `workspace://changed`, and
 * registry connectors on `connectors://changed`.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { connectorsQueryKeys } from '@/hooks/useConnectors'
import { useSignInFlow } from '@/hooks/useEmail'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'
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
  return useQuery({
    queryKey: hubKeys.grants,
    queryFn: async () => unwrapResult(await client.listConnectorGrants()),
  })
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
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (change: GrantChange) =>
      unwrapResult(
        change.kind === 'grant'
          ? await client.grantConnectorAccess(
              change.agentId,
              change.connectorId,
              change.access,
              change.scope
            )
          : await client.revokeConnectorAccess(change.grant)
      ),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: hubKeys.grants }),
  })
}

/** Calendars and drives, and the calendar check */
export function useWorkspaceStatus() {
  useHubEvents()
  return useQuery({
    queryKey: hubKeys.workspace,
    queryFn: async () => unwrapResult(await client.getWorkspaceStatus()),
  })
}

export function useWorkspaceAction() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (
      action:
        | { kind: 'check' }
        | { kind: 'interval'; minutes: number }
        | { kind: 'disconnect'; account: AccountKind }
    ): Promise<WorkspaceStatus> => {
      switch (action.kind) {
        case 'check':
          return unwrapResult(await client.checkCalendarNow())
        case 'interval':
          return unwrapResult(await client.setCalendarInterval(action.minutes))
        case 'disconnect':
          return unwrapResult(await client.disconnectWorkspace(action.account))
      }
    },
    onSuccess: status => {
      queryClient.setQueryData(hubKeys.workspace, status)
      void queryClient.invalidateQueries({ queryKey: hubKeys.grants })
    },
  })
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
  return useQuery({
    queryKey: hubKeys.registry,
    queryFn: async () =>
      unwrapResult(await client.listRegistryConnectors(false)),
  })
}

/** What installing one would mean, for the consent dialog */
export function useConnectorConsent(
  target: { registryUrl: string; connectorId: string } | null
) {
  return useQuery({
    queryKey: [
      ...hubKeys.registry,
      'consent',
      target?.registryUrl ?? '',
      target?.connectorId ?? '',
    ],
    enabled: target !== null,
    queryFn: async () =>
      unwrapResult(
        await client.getConnectorConsent(
          target?.registryUrl ?? '',
          target?.connectorId ?? ''
        )
      ),
  })
}

export function useInstallRegistryConnector() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (input: {
      registryUrl: string
      connectorId: string
      apiKey: string | null
    }) =>
      unwrapResult(
        await client.installRegistryConnector(
          input.registryUrl,
          input.connectorId,
          input.apiKey
        )
      ),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: connectorsQueryKeys.all }),
  })
}

export function useRemoveConnector() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) =>
      unwrapResult(await client.removeConnector(id)),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: connectorsQueryKeys.all }),
  })
}

export function useScaffoldConnector() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (input: ScaffoldConnectorInput) =>
      unwrapResult(await client.scaffoldConnector(input)),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: connectorsQueryKeys.all }),
  })
}

export function usePublishConnector() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (input: PublishConnectorInput) =>
      unwrapResult(await client.publishConnector(input)),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: connectorsQueryKeys.all }),
  })
}
