/**
 * The mascot: its name (shared with the CLI, in ~/.nandi/config.json), its
 * pose from what the app is doing, and the hidden phrase.
 */
import { useEffect, useState } from 'react'
import { toast } from 'sonner'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { isMascotPhrase, nandiState } from '@/lib/mascot'
import { unwrapResult } from '@/lib/tauri-bindings'
import { useAgentStore } from '@/store/agent-store'
import { useUIStore } from '@/store/ui-store'
import { useAllPendingApprovals } from './useInbox'

export const mascotKeys = { info: ['mascot'] as const }

export function useMascot() {
  return useQuery({
    queryKey: mascotKeys.info,
    queryFn: async () => unwrapResult(await client.mascotGet()),
    staleTime: Infinity,
  })
}

/** The bull's name, Nandi until it's loaded or renamed */
export function useMascotName(): string {
  return useMascot().data?.name ?? 'Nandi'
}

export function useRenameMascot() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (name: string | null) =>
      unwrapResult(await client.mascotSet(name)),
    onSuccess: info => queryClient.setQueryData(mascotKeys.info, info),
  })
}

/**
 * `wait` while approvals are pending, `carry` while agents run, `done`
 * for a moment after an approval, else `rest`
 */
export function useNandiState() {
  const approvals = useAllPendingApprovals().data?.length ?? 0
  const running = useAgentStore(
    state => state.dashboard?.stats.runningAgents ?? 0
  )
  const doneUntil = useUIStore(state => state.mascotDoneUntil)
  const [now, setNow] = useState(() => Date.now())

  // An approval granted anywhere: a moment of `done`
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (
        event.event === 'agent://approval_resolved' &&
        event.payload.status === 'approved'
      ) {
        useUIStore.getState().celebrateMascot()
      }
    })
    return () => subscription.unsubscribe()
  }, [])

  // Re-render when `done` should end
  useEffect(() => {
    const left = doneUntil - Date.now()
    if (left <= 0) return
    const timer = window.setTimeout(() => setNow(Date.now()), left + 10)
    return () => window.clearTimeout(timer)
  }, [doneUntil])

  return nandiState({
    pendingApprovals: approvals,
    runningAgents: running,
    doneUntil,
    now,
  })
}

/**
 * Handles the hidden phrase before normal parsing: when `input` is it,
 * the bull answers (a toast and a moment of `done`) and this returns
 * true, so the caller does nothing else with the input.
 */
export function handlePhrase(input: string): boolean {
  if (!isMascotPhrase(input)) return false
  void client.mascotPhrase(input).then(result => {
    if (result.status === 'ok' && result.data) {
      useUIStore.getState().celebrateMascot(2500)
      toast(result.data.line)
    }
  })
  return true
}
