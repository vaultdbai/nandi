/**
 * Trigger user actions. Results arrive back as `trigger://*` events, which
 * refresh the trigger queries (see `agent-sync`); each action also
 * invalidates what it changed, in case this client missed the event.
 */

import { toast } from 'sonner'
import i18n from '@/i18n/config'
import { client } from '@/lib/client'
import { logger } from '@/lib/logger'
import { queryClient } from '@/lib/query-client'
import type {
  JsonValue,
  SavedTrigger,
  TriggerFire,
  TriggerInput,
} from '@/lib/tauri-bindings'
import { useNavigationStore } from '@/store/navigation-store'

function refresh(triggerId?: string) {
  void queryClient.invalidateQueries({ queryKey: ['triggers'] })
  if (triggerId) {
    void queryClient.invalidateQueries({ queryKey: ['trigger', triggerId] })
    void queryClient.invalidateQueries({
      queryKey: ['triggerFires', triggerId],
    })
  }
}

/** Returns the saved trigger (with its one-time secret), or null. */
export async function createTrigger(
  input: TriggerInput
): Promise<SavedTrigger | null> {
  const result = await client.createTrigger(input)
  if (result.status === 'error') {
    logger.warn('Trigger create failed', { error: result.error })
    toast.error(result.error)
    return null
  }
  refresh(result.data.trigger.id)
  toast.success(i18n.t('triggers.created', { name: result.data.trigger.name }))
  return result.data
}

export async function updateTrigger(
  id: string,
  input: TriggerInput
): Promise<SavedTrigger | null> {
  const result = await client.updateTrigger(id, input)
  if (result.status === 'error') {
    toast.error(result.error)
    return null
  }
  refresh(id)
  toast.success(i18n.t('triggers.saved'))
  return result.data
}

export async function deleteTrigger(id: string): Promise<boolean> {
  const result = await client.deleteTrigger(id)
  if (result.status === 'error') {
    toast.error(result.error)
    return false
  }
  refresh(id)
  useNavigationStore
    .getState()
    .navigate({ kind: 'automations', tab: 'triggers' })
  return true
}

/**
 * Turns a trigger on or off. On failure the switch reverts (the queries
 * refresh) and a toast says so in plain words, with the reason as detail.
 */
export async function setTriggerEnabled(
  id: string,
  enabled: boolean
): Promise<boolean> {
  const result = await client.setTriggerEnabled(id, enabled)
  refresh(id)
  if (result.status === 'error') {
    toast.error(
      i18n.t(enabled ? 'triggers.turnOnFailed' : 'triggers.turnOffFailed'),
      { description: result.error }
    )
    return false
  }
  return true
}

/** A new secret, shown once; null on failure */
export async function rotateTriggerSecret(id: string): Promise<string | null> {
  const result = await client.rotateTriggerSecret(id)
  if (result.status === 'error') {
    toast.error(result.error)
    return null
  }
  refresh(id)
  return result.data.secret
}

export async function testFireTrigger(
  id: string,
  payload: unknown
): Promise<TriggerFire | null> {
  // `payload` is parsed JSON (or undefined for none)
  const result = await client.testFireTrigger(
    id,
    (payload ?? null) as JsonValue | null
  )
  refresh(id)
  if (result.status === 'error') {
    toast.error(result.error)
    return null
  }
  const fire = result.data
  if (fire.status === 'queued') toast.success(i18n.t('triggers.testFired'))
  else toast.warning(fire.error ?? i18n.t(`triggers.fireStatus.${fire.status}`))
  return fire
}
