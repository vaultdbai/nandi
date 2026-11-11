/**
 * Getting to know you (`services::knowme`): the know-me to-do in the
 * Inbox, the groups a personal answer may be shared with, and the latest
 * confirmed changes. Kept live by `knowme://changed`. Works with no relay.
 */
import { useQueryClient } from '@tanstack/react-query'
import type { KnowMeProposal, KnowMeTodoAction } from '@/lib/bindings'
import { client, type NandiEventName } from '@/lib/client'
import { useInvalidateOn } from '@/hooks/useInvalidateOn'
import { useClientQuery, useClientMutation } from '@/hooks/useClientQuery'

export const knowMeKeys = {
  status: ['knowMe', 'status'] as const,
}

/** The one know-me chat (`knowme::THREAD_ID`) */
export const KNOW_ME_THREAD = 'inbox:knowMe:you'

const KNOW_ME_EVENTS: readonly NandiEventName[] = [
  'knowme://changed',
  'agent://approval_resolved',
  'network://group_changed',
]

export function useKnowMe() {
  useInvalidateOn(KNOW_ME_EVENTS, knowMeKeys.status)

  return useClientQuery(knowMeKeys.status, () => client.getKnowMe())
}

/** Later (back in a week) or Skip (back in a month) */
export function useKnowMeTodo() {
  const queryClient = useQueryClient()
  return useClientMutation(
    (action: KnowMeTodoAction) => client.knowMeTodo(action),
    {
      onSuccess: status => queryClient.setQueryData(knowMeKeys.status, status),
    }
  )
}

/** Confirms a know-me proposal card, as edited */
export function useConfirmKnowMeProposal() {
  return useClientMutation(
    ({
      approvalId,
      proposal,
    }: {
      approvalId: string
      proposal: KnowMeProposal
    }) => client.confirmKnowMeProposal(approvalId, proposal),
    { invalidates: knowMeKeys.status }
  )
}
