/**
 * The weekly retro's Inbox items (`services::retro`): proposals waiting
 * for a yes, changes applied this week with their Undo, and changes the
 * check says didn't help. Kept live by `retro://changed`. Works with no
 * relay.
 */
import type { RetroAction } from '@/lib/bindings'
import { client, type NandiEventName } from '@/lib/client'
import { useInvalidateOn } from '@/hooks/useInvalidateOn'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

export const retroKeys = {
  items: ['retro', 'items'] as const,
}

const RETRO_EVENTS: readonly NandiEventName[] = [
  'retro://changed',
  // A decision can settle the check's verdict
  'agent://approval_resolved',
]

export function useRetroItems() {
  useInvalidateOn(RETRO_EVENTS, retroKeys.items)

  return useClientQuery(retroKeys.items, () => client.listRetroItems())
}

/** Apply (one gold copy commit), refuse, undo or keep a proposal */
export function useRetroAction() {
  return useClientMutation(
    ({ id, action }: { id: string; action: RetroAction }) =>
      client.actOnRetroProposal(id, action),
    { invalidates: retroKeys.items }
  )
}
