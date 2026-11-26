import { client } from '@/lib/client'
import { useClientQuery } from '@/hooks/useClientQuery'
import type { JsonValue } from '@/lib/bindings'

/** What a save call would create and may reach (`preview_save`) */
export function useSavePreview(
  approvalId: string,
  args: JsonValue,
  enabled = true
) {
  return useClientQuery(
    ['savePreview', approvalId],
    () => client.previewSave(args),
    { staleTime: Infinity, enabled }
  )
}
