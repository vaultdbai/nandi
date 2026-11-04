/**
 * `.nandi/llm.yaml`, the team's shared LLM config (wave EF): what the repo
 * holds and what's in effect after env > repo > local. Kept live by
 * `config://repo_llm_changed`.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { RepoLlmConfigStatus } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

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
  return useQuery({
    queryKey: repoConfigQueryKey,
    queryFn: async () => unwrapResult(await client.getRepoLlmConfig()),
  })
}

/** Re-reads `.nandi/llm.yaml` now ("Sync") */
export function useSyncRepoLlmConfig() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async () => unwrapResult(await client.reloadRepoLlmConfig()),
    onSuccess: status => queryClient.setQueryData(repoConfigQueryKey, status),
  })
}
