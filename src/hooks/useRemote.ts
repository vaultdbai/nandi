/**
 * The optional git remote: status, configure, push, pull. Kept live by
 * `remote://*` events. Pushes only ever go to `<prefix>/<slug>`.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { RemoteCredential, RemoteStatus } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

/** Everything remote-related lives under `['remote']` */
export const remoteQueryKeys = {
  all: ['remote'] as const,
  status: ['remote', 'status'] as const,
  branches: ['remote', 'branches'] as const,
}

const REMOTE_EVENTS = new Set<string>([
  'remote://status_changed',
  'remote://push_completed',
  'remote://pull_completed',
  'remote://error',
  'nandi://resync',
])

export interface ConfigureRemoteInput {
  url: string
  /** `null` keeps the stored credential */
  credential: RemoteCredential | null
  prefix?: string | null
  slug?: string | null
}

export function useRemoteStatus() {
  const queryClient = useQueryClient()

  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (REMOTE_EVENTS.has(event.event)) {
        void queryClient.invalidateQueries({ queryKey: remoteQueryKeys.all })
      } else if (event.event === 'identity://changed') {
        // The slug names the target branch
        void queryClient.invalidateQueries({
          queryKey: remoteQueryKeys.status,
        })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return useQuery<RemoteStatus>({
    queryKey: remoteQueryKeys.status,
    queryFn: async () => unwrapResult(await client.getRemoteStatus()),
  })
}

/** Branch names on the remote (only when one is configured) */
export function useRemoteBranches(enabled = true) {
  return useQuery<string[]>({
    queryKey: remoteQueryKeys.branches,
    queryFn: async () => unwrapResult(await client.listRemoteBranches()),
    enabled,
  })
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
  return useMutation({
    mutationFn: async ({
      url,
      credential,
      prefix,
      slug,
    }: ConfigureRemoteInput) =>
      unwrapResult(
        await client.configureRemote(
          url.trim(),
          credential,
          prefix?.trim() || null,
          slug?.trim() || null
        )
      ),
    onSuccess: status => {
      queryClient.setQueryData(remoteQueryKeys.status, status)
      invalidate()
    },
  })
}

/** Forgets the remote and its credential */
export function useClearRemote() {
  const invalidate = useInvalidateRemote()
  return useMutation({
    mutationFn: async () => unwrapResult(await client.clearRemote()),
    onSuccess: invalidate,
  })
}

export function useTestRemoteConnection() {
  return useMutation({
    mutationFn: async () => unwrapResult(await client.testRemoteConnection()),
  })
}

/**
 * Pushes to `<prefix>/<slug>`. A result with `conflict` pushed nothing:
 * pull first, or push again with `overwrite`.
 */
export function usePushToRemote() {
  const invalidate = useInvalidateRemote()
  return useMutation({
    mutationFn: async (overwrite?: boolean) =>
      unwrapResult(await client.pushToRemote(overwrite ?? false)),
    onSettled: invalidate,
  })
}

/** A result with `conflicts` left a merge in progress (nothing resolved) */
export function usePullFromRemote() {
  const invalidate = useInvalidateRemote()
  return useMutation({
    mutationFn: async () => unwrapResult(await client.pullFromRemote()),
    onSettled: invalidate,
  })
}
