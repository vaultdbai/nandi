/**
 * Local identity: display name and user slug (`~/.nandi/repo/NANDI.md`).
 * Kept live by `identity://changed`.
 */
import { useEffect } from 'react'
import { useQueryClient, type QueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

export const identityQueryKeys = {
  all: ['identity'] as const,
  status: ['identity', 'status'] as const,
  verified: ['identity', 'verified'] as const,
}

/**
 * This Nandi's person, locked from the first mailbox connected (read-only;
 * `null` before any). Refreshed when a mailbox connects.
 */
export function useVerifiedIdentity() {
  const queryClient = useQueryClient()

  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (event.event === 'email://changed') {
        void queryClient.invalidateQueries({
          queryKey: identityQueryKeys.verified,
        })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return useClientQuery(identityQueryKeys.verified, () =>
    client.getVerifiedIdentity()
  )
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

  return useClientQuery(identityQueryKeys.status, () =>
    client.getIdentityStatus()
  )
}

/** The slug (and so the push branch) can change with the name */
function invalidateIdentity(queryClient: QueryClient) {
  void queryClient.invalidateQueries({ queryKey: identityQueryKeys.all })
  void queryClient.invalidateQueries({ queryKey: ['remote'] })
}

/** Writes the display name to NANDI.md; derives the slug if none is set */
export function useSetDisplayName() {
  const queryClient = useQueryClient()
  return useClientMutation(
    (name: string) => client.setDisplayName(name.trim()),
    { onSuccess: () => invalidateIdentity(queryClient) }
  )
}

/** `^[a-z0-9][a-z0-9-]{0,30}[a-z0-9]$`; names the branch pushes go to */
export function useSetUserSlug() {
  const queryClient = useQueryClient()
  return useClientMutation((slug: string) => client.setUserSlug(slug.trim()), {
    onSuccess: () => invalidateIdentity(queryClient),
  })
}
