import { useQuery } from '@tanstack/react-query'
import { client } from '@/lib/client'
import { unwrapResult } from '@/lib/tauri-bindings'

/** Every topic of the built-in help guide (fixed for the app's life) */
export function useHelpTopics() {
  return useQuery({
    queryKey: ['helpTopics'],
    queryFn: async () => unwrapResult(await client.helpTopics()),
    staleTime: Infinity,
  })
}
