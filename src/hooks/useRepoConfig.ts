/**
 * `.nandi/llm.yaml`, the team's shared LLM config: what the repo
 * holds and what's in effect after env > repo > local. Kept live by
 * `config://repo_llm_changed`.
 */
import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { RepoLlmConfigStatus } from '@/lib/bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

export const repoConfigQueryKey = ['repo-config', 'llm'] as const

export function useRepoLlmConfig() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (event.event === 'config://repo_llm_changed') {
        queryClient.setQueryData(
          repoConfigQueryKey,
          event.payload as RepoLlmConfigStatus
        )
      }
    })
    return subscription.unsubscribe
  }, [queryClient])
  return useClientQuery(repoConfigQueryKey, () => client.getRepoLlmConfig())
}

/** Re-reads `.nandi/llm.yaml` now ("Sync") */
export function useSyncRepoLlmConfig() {
  const queryClient = useQueryClient()
  return useClientMutation(() => client.reloadRepoLlmConfig(), {
    onSuccess: status => queryClient.setQueryData(repoConfigQueryKey, status),
  })
}
