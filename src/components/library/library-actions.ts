/**
 * Library actions with side effects (install with consent, update).
 * Toasts on failure; callers only learn whether it worked.
 */
import { toast } from 'sonner'
import i18n from '@/i18n/config'
import { client } from '@/lib/client'
import type { ManifestCapabilities } from '@/lib/bindings'
import {
  applyUpdate,
  checkForUpdates,
  grantConsent,
  installFromMarketplace,
  installLocalPack,
} from '@/lib/marketplace-actions'
import { approvedAtInstall } from './library-model'

/**
 * Installs an agent or pack, then records consent for what the dialog the
 * user just confirmed listed, so the first run doesn't ask again for it
 * (§6.1). Anything it didn't list, and a later version asking for more,
 * still gets the first-run consent, for that access only.
 */
export async function installAndAllow(input: {
  kind: 'agent' | 'pack'
  id: string
  name: string
  /** The agents it installs (an agent: itself) */
  agentIds: string[]
  /** A pack from a folder or `.zip` on this computer, not a registry */
  localPath?: string
  /** The access the install dialog listed */
  shown: ManifestCapabilities
}): Promise<boolean> {
  const installed =
    input.localPath !== undefined
      ? await installLocalPack(input.localPath, input.name)
      : await installFromMarketplace(input.kind, input.id, input.name)
  if (!installed) return false
  for (const agentId of input.agentIds) {
    const status = await client.getAgentConsent(agentId)
    if (status.status === 'error' || !status.data.required) continue
    const approved = approvedAtInstall(status.data.categories, input.shown)
    if (approved.length > 0) await grantConsent(agentId, approved)
  }
  return true
}

/** Finds the newer version of one installed item and applies it */
export async function updateItem(id: string, name: string): Promise<boolean> {
  const updates = await checkForUpdates()
  const update = updates?.find(candidate => candidate.id === id)
  if (!update) {
    if (updates) toast.info(i18n.t('library.upToDate', { name }))
    return false
  }
  await applyUpdate(update, name)
  return true
}
