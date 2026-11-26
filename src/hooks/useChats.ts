import { client } from '@/lib/client'
import { useClientQuery } from '@/hooks/useClientQuery'

/**
 * The person's chats with their agents, latest first (the Nandi home).
 * Under `globalRuns`: a run starting or ending is what changes a chat, so
 * the run events that refresh the runs refresh these too (`agent-sync`).
 */
export const chatsQueryKey = ['globalRuns', 'chats'] as const

export function useChats() {
  return useClientQuery(chatsQueryKey, () => client.listChats())
}
