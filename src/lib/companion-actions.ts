import { toast } from 'sonner'
import { client, isDesktop } from '@/lib/client'
import { useCompanionStore } from '@/store/companion-store'

/**
 * Watch in companion: pins the Request at the top of the companion's
 * Running and shows it. On the desktop the companion is its own window
 * (`companion_focus_request`); on the web it shares this page's store.
 */
export async function watchInCompanion(requestId: string): Promise<void> {
  useCompanionStore.getState().focusRequest(requestId)
  if (!isDesktop()) return
  const result = await client.companionFocusRequest(requestId)
  if (result.status === 'error') toast.error(result.error)
}
