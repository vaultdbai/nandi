import { toast } from 'sonner'
import { marketplaceQueryKeys } from '@/hooks/useMarketplace'
import i18n from '@/i18n/config'
import { client } from '@/lib/client'
import { logger } from '@/lib/logger'
import { queryClient } from '@/lib/query-client'
import type {
  ApprovedCategory,
  CommunityRegistry,
  ConsentStatus,
  PublishAgentInput,
  PublishResult,
  UpdateAvailable,
} from '@/lib/bindings'
import { installKey, useMarketplaceStore } from '@/store/marketplace-store'
import { useNavigationStore } from '@/store/navigation-store'

export type InstallStatus =
  | { state: 'queued' }
  | { state: 'installing' }
  | { state: 'installed' }
  | { state: 'failed'; error: string }

/**
 * Installs packs one after another (each is a git fetch; doing them in
 * parallel just splits the bandwidth). Reports each pack's status as it
 * goes. Returns whether all of them installed.
 */
export async function installPacks(
  packIds: string[],
  onStatus: (packId: string, status: InstallStatus) => void
): Promise<boolean> {
  for (const packId of packIds) onStatus(packId, { state: 'queued' })
  let allInstalled = true
  for (const packId of packIds) {
    onStatus(packId, { state: 'installing' })
    const result = await client.installPack(packId)
    useMarketplaceStore.getState().clearProgress(installKey('pack', packId))
    if (result.status === 'ok') {
      onStatus(packId, { state: 'installed' })
    } else {
      allInstalled = false
      logger.warn('Pack install failed', { packId, error: result.error })
      onStatus(packId, { state: 'failed', error: result.error })
    }
  }
  void queryClient.invalidateQueries({ queryKey: marketplaceQueryKeys.all })
  return allInstalled
}

/**
 * Installs one pack or agent from the Discover tab, with a toast either
 * way. Marks it busy (in `useMarketplaceStore`) while it runs, so every
 * card and dialog showing it agrees. Returns whether it installed.
 */
export async function installFromMarketplace(
  kind: 'pack' | 'agent',
  id: string,
  name: string
): Promise<boolean> {
  const key = installKey(kind, id)
  const store = useMarketplaceStore.getState()
  store.setBusy(key, true)
  const result =
    kind === 'pack'
      ? await client.installPack(id)
      : await client.installAgent(id)
  store.setBusy(key, false)
  store.clearProgress(key)
  void queryClient.invalidateQueries({ queryKey: marketplaceQueryKeys.all })
  if (result.status === 'ok') {
    toast.success(i18n.t('marketplace.installedToast', { name }))
    return true
  }
  logger.warn('Install failed', { kind, id, error: result.error })
  toast.error(i18n.t('marketplace.installFailed', { name }), {
    description: result.error,
  })
  return false
}

/**
 * Installs the pack in a local folder or `.zip` (no internet, no relay),
 * with a toast either way. Returns whether it installed.
 */
export async function installLocalPack(
  path: string,
  name: string
): Promise<boolean> {
  const result = await client.installLocalPack(path)
  void queryClient.invalidateQueries({ queryKey: marketplaceQueryKeys.all })
  if (result.status === 'ok') {
    toast.success(i18n.t('marketplace.installedToast', { name }))
    return true
  }
  logger.warn('Local pack install failed', { path, error: result.error })
  toast.error(i18n.t('marketplace.installFailed', { name }), {
    description: result.error,
  })
  return false
}

/** Busy key for an update (shares `busy` with installs) */
export const updateKey = (id: string) => installKey('update', id)

/** Asks every installed source for newer versions (never applies them). */
export async function checkForUpdates(): Promise<UpdateAvailable[] | null> {
  const result = await client.checkForUpdates()
  if (result.status === 'error') {
    toast.error(i18n.t('marketplace.checkFailed'), {
      description: result.error,
    })
    return null
  }
  if (result.data.length === 0) {
    toast.success(i18n.t('marketplace.upToDate'))
  }
  return result.data
}

/** Applies one update (the user asked). Returns whether it applied. */
export async function applyUpdate(update: UpdateAvailable, name: string) {
  const store = useMarketplaceStore.getState()
  store.setBusy(updateKey(update.id), true)
  const result = await client.applyUpdate(update.id, update.latestRef)
  store.setBusy(updateKey(update.id), false)
  store.clearProgress(installKey(update.kind, update.id))
  if (result.status === 'error') {
    toast.error(i18n.t('marketplace.updateFailed', { name }), {
      description: result.error,
    })
    return false
  }
  void queryClient.invalidateQueries({ queryKey: marketplaceQueryKeys.all })
  toast.success(
    i18n.t('marketplace.updated', { name, version: result.data.toVersion })
  )
  return true
}

export async function uninstallInstalled(
  kind: string,
  id: string,
  name: string
): Promise<boolean> {
  const result = await client.uninstallInstalled(kind, id)
  void queryClient.invalidateQueries({ queryKey: marketplaceQueryKeys.all })
  if (result.status === 'error') {
    toast.error(i18n.t('marketplace.uninstallFailed', { name }), {
      description: result.error,
    })
    return false
  }
  toast.success(i18n.t('marketplace.uninstalled', { name }))
  return true
}

/**
 * Forks a marketplace agent into the user's own agents and opens it.
 * Anything that still names the original is shown as a warning.
 */
export async function forkAgent(
  installedId: string,
  newId: string,
  newName: string
): Promise<string | null> {
  const result = await client.forkInstalledAgent(installedId, newId, newName)
  void queryClient.invalidateQueries({ queryKey: marketplaceQueryKeys.all })
  if (result.status === 'error') return result.error
  const { agent, warnings } = result.data
  toast.success(i18n.t('marketplace.forked', { name: agent.name }), {
    description: warnings.length > 0 ? warnings.join('\n') : undefined,
  })
  void queryClient.invalidateQueries({ queryKey: ['agents'] })
  useNavigationStore.getState().navigate({ kind: 'agent', agentId: agent.id })
  return null
}

/**
 * Approves capability categories for a marketplace agent's first run, with
 * the items the user was shown: nothing else is approved. Returns the new
 * status, or `null` if it couldn't be saved.
 */
export async function grantConsent(
  agentId: string,
  approved: ApprovedCategory[]
): Promise<ConsentStatus | null> {
  const result = await client.grantAgentConsent(agentId, approved)
  if (result.status === 'error') {
    toast.error(i18n.t('marketplace.consentFailed'), {
      description: result.error,
    })
    return null
  }
  return result.data
}

/** Adds a registry. Returns `null` on success, else the error. */
export async function addRegistry(input: {
  url: string
  kind: string
  token: string | null
  useGithubSignIn: boolean
}): Promise<string | null> {
  const result = await client.addRegistry(input)
  if (result.status === 'error') return result.error
  void queryClient.invalidateQueries({ queryKey: marketplaceQueryKeys.all })
  toast.success(i18n.t('marketplace.registryAdded', { url: result.data.url }))
  return null
}

export async function removeRegistry(url: string): Promise<boolean> {
  const result = await client.removeRegistry(url)
  void queryClient.invalidateQueries({ queryKey: marketplaceQueryKeys.all })
  if (result.status === 'error') {
    toast.error(i18n.t('marketplace.registryRemoveFailed'), {
      description: result.error,
    })
    return false
  }
  return true
}

/** Creates (or finds) the user's private community registry. */
export async function createCommunityRegistry(): Promise<CommunityRegistry | null> {
  const result = await client.createCommunityRegistry()
  void queryClient.invalidateQueries({ queryKey: marketplaceQueryKeys.all })
  if (result.status === 'error') {
    toast.error(i18n.t('marketplace.communityFailed'), {
      description: result.error,
    })
    return null
  }
  return result.data
}

/** Publishes a local agent. Returns the result, or the error message. */
export async function publishAgent(
  input: PublishAgentInput
): Promise<PublishResult | string> {
  const result = await client.publishAgent(input)
  if (result.status === 'error') return result.error
  void queryClient.invalidateQueries({ queryKey: marketplaceQueryKeys.all })
  return result.data
}
