/**
 * Local identity: display name and user slug (`~/.nandi/repo/NANDI.md`).
 * Kept live by `identity://changed`.
 */
import { useEffect } from 'react'
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { IdentityStatus } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

export const identityQueryKeys = {
  all: ['identity'] as const,
  status: ['identity', 'status'] as const,
}

export function useIdentity() {
  const queryClient = useQueryClient()

  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (event.event === 'identity://changed') {
        void queryClient.invalidateQueries({ queryKey: identityQueryKeys.all })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return useQuery<IdentityStatus>({
    queryKey: identityQueryKeys.status,
    queryFn: async () => unwrapResult(await client.getIdentityStatus()),
  })
}

/** The slug (and so the push branch) can change with the name */
function invalidateIdentity(queryClient: QueryClient) {
  void queryClient.invalidateQueries({ queryKey: identityQueryKeys.all })
  void queryClient.invalidateQueries({ queryKey: ['remote'] })
}

/** Writes the display name to NANDI.md; derives the slug if none is set */
export function useSetDisplayName() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (name: string) =>
      unwrapResult(await client.setDisplayName(name.trim())),
    onSuccess: () => invalidateIdentity(queryClient),
  })
}

/** `^[a-z0-9][a-z0-9-]{0,30}[a-z0-9]$`; names the branch pushes go to */
export function useSetUserSlug() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (slug: string) =>
      unwrapResult(await client.setUserSlug(slug.trim())),
    onSuccess: () => invalidateIdentity(queryClient),
  })
}
