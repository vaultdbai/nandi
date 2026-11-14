import { client } from '@/lib/client'
import { useClientQuery } from '@/hooks/useClientQuery'

/** Every topic of the built-in help guide (fixed for the app's life) */
export function useHelpTopics() {
  return useClientQuery(['helpTopics'], () => client.helpTopics(), {
    staleTime: Infinity,
  })
}
