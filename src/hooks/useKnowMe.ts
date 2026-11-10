/**
 * Getting to know you (`services::knowme`): the know-me to-do in the
 * Inbox, the groups a personal answer may be shared with, and the latest
 * confirmed changes. Kept live by `knowme://changed`. Works with no relay.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { KnowMeProposal, KnowMeTodoAction } from '@/lib/bindings'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'

export const knowMeKeys = {
  status: ['knowMe', 'status'] as const,
}

/** The one know-me chat (`knowme::THREAD_ID`) */
export const KNOW_ME_THREAD = 'inbox:knowMe:you'

const EVENTS = new Set([
  'knowme://changed',
  'agent://approval_resolved',
  'network://group_changed',
  'nandi://resync',
])

export function useKnowMe() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (EVENTS.has(event.event)) {
        void queryClient.invalidateQueries({ queryKey: knowMeKeys.status })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return useQuery({
    queryKey: knowMeKeys.status,
    queryFn: async () => unwrapResult(await client.getKnowMe()),
  })
}

/** Later (back in a week) or Skip (back in a month) */
export function useKnowMeTodo() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (action: KnowMeTodoAction) =>
      unwrapResult(await client.knowMeTodo(action)),
    onSuccess: status => queryClient.setQueryData(knowMeKeys.status, status),
  })
}

/** Confirms a know-me proposal card, as edited */
export function useConfirmKnowMeProposal() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      approvalId,
      proposal,
    }: {
      approvalId: string
      proposal: KnowMeProposal
    }) =>
      unwrapResult(await client.confirmKnowMeProposal(approvalId, proposal)),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: knowMeKeys.status }),
  })
}
