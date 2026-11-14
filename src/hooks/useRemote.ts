/**
 * The optional git remote: status, configure, push, pull. Kept live by
 * `remote://*` events. Pushes only ever go to `<prefix>/<slug>`.
 */
import { useQueryClient } from '@tanstack/react-query'
import { client, type NandiEventName } from '@/lib/client'
import { useInvalidateOn } from '@/hooks/useInvalidateOn'
import type { RemoteCredential } from '@/lib/bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

/** Everything remote-related lives under `['remote']` */
export const remoteQueryKeys = {
  all: ['remote'] as const,
  status: ['remote', 'status'] as const,
  branches: ['remote', 'branches'] as const,
}

const REMOTE_EVENTS: readonly NandiEventName[] = [
  'remote://status_changed',
  'remote://push_completed',
  'remote://pull_completed',
  'remote://error',
]

const IDENTITY_EVENTS: readonly NandiEventName[] = ['identity://changed']

export interface ConfigureRemoteInput {
  url: string
  /** `null` keeps the stored credential */
  credential: RemoteCredential | null
  prefix?: string | null
  slug?: string | null
}

export function useRemoteStatus() {
  useInvalidateOn(REMOTE_EVENTS, remoteQueryKeys.all)
  // The slug names the target branch
  useInvalidateOn(IDENTITY_EVENTS, remoteQueryKeys.status)

  return useClientQuery(remoteQueryKeys.status, () => client.getRemoteStatus())
}

/** Branch names on the remote (only when one is configured) */
export function useRemoteBranches(enabled = true) {
  return useClientQuery(
    remoteQueryKeys.branches,
    () => client.listRemoteBranches(),
    { enabled }
  )
}

function useInvalidateRemote() {
  const queryClient = useQueryClient()
  return () => {
    void queryClient.invalidateQueries({ queryKey: remoteQueryKeys.all })
    void queryClient.invalidateQueries({ queryKey: ['identity'] })
  }
}

export function useConfigureRemote() {
  const queryClient = useQueryClient()
  const invalidate = useInvalidateRemote()
  return useClientMutation(
    ({ url, credential, prefix, slug }: ConfigureRemoteInput) =>
      client.configureRemote(
        url.trim(),
        credential,
        prefix?.trim() || null,
        slug?.trim() || null
      ),
    {
      onSuccess: status => {
        queryClient.setQueryData(remoteQueryKeys.status, status)
        invalidate()
      },
    }
  )
}

/** Forgets the remote and its credential */
export function useClearRemote() {
  const invalidate = useInvalidateRemote()
  return useClientMutation(() => client.clearRemote(), {
    onSuccess: invalidate,
  })
}

export function useTestRemoteConnection() {
  return useClientMutation(() => client.testRemoteConnection())
}

/**
 * Pushes to `<prefix>/<slug>`. A result with `conflict` pushed nothing:
 * pull first, or push again with `overwrite`.
 */
export function usePushToRemote() {
  const invalidate = useInvalidateRemote()
  return useClientMutation(
    (overwrite?: boolean) => client.pushToRemote(overwrite ?? false),
    { onSettled: invalidate }
  )
}

/** A result with `conflicts` left a merge in progress (nothing resolved) */
export function usePullFromRemote() {
  const invalidate = useInvalidateRemote()
  return useClientMutation(() => client.pullFromRemote(), {
    onSettled: invalidate,
  })
}
