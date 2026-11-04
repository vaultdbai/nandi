/**
 * The GitHub sign-in: one token (kept in the OS keyring) for every
 * github.com repository Nandi uses — the GitHub connector, Backup, team
 * and capability registries, marketplace sources, and publishing.
 */
import { useEffect, useState } from 'react'
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { DeviceFlowState, GithubSignIn } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'
import { logger } from '@/lib/logger'
import { connectorsQueryKeys } from './useConnectors'

export const githubSignInQueryKeys = {
  all: ['github', 'sign-in'] as const,
}

export function useGithubSignIn() {
  return useQuery<GithubSignIn>({
    queryKey: githubSignInQueryKeys.all,
    queryFn: async () => unwrapResult(await client.getGithubSignIn()),
  })
}

/** Everything that reads with the sign-in */
function invalidateSignIn(queryClient: QueryClient, status: GithubSignIn) {
  queryClient.setQueryData(githubSignInQueryKeys.all, status)
  void queryClient.invalidateQueries({ queryKey: connectorsQueryKeys.all })
  void queryClient.invalidateQueries({ queryKey: ['remote'] })
}

export type GithubSignInFlow =
  | { phase: 'idle' }
  | { phase: 'waiting'; flow: DeviceFlowState }
  | { phase: 'error'; message: string }

/**
 * `start()` shows a code and opens github.com/login/device; while waiting,
 * the core asks GitHub every `flow.interval` seconds.
 */
export function useGithubSignInFlow() {
  const queryClient = useQueryClient()
  const [state, setState] = useState<GithubSignInFlow>({ phase: 'idle' })

  const flow = state.phase === 'waiting' ? state.flow : null
  useEffect(() => {
    if (!flow) return
    let cancelled = false
    const timer = window.setInterval(
      async () => {
        const result = await client.pollGithubSignIn()
        if (cancelled) return
        if (result.status === 'error') {
          setState({ phase: 'error', message: result.error })
        } else if (result.data.signedIn) {
          setState({ phase: 'idle' })
          invalidateSignIn(queryClient, result.data)
        }
      },
      Math.max(flow.interval, 1) * 1000
    )
    return () => {
      cancelled = true
      window.clearInterval(timer)
    }
  }, [flow, queryClient])

  const start = async () => {
    const result = await client.startGithubSignIn()
    if (result.status === 'error') {
      setState({ phase: 'error', message: result.error })
      return
    }
    setState({ phase: 'waiting', flow: result.data })
    client
      .openExternal(result.data.verificationUri)
      .catch((error: unknown) =>
        logger.warn('Could not open the GitHub sign-in page', { error })
      )
  }

  const cancel = () => setState({ phase: 'idle' })

  return { state, start, cancel }
}

export function useSignOutGithub() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async () => unwrapResult(await client.signOutGithub()),
    onSuccess: status => invalidateSignIn(queryClient, status),
  })
}
