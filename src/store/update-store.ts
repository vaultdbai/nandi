import { create } from 'zustand'
import { devtools } from 'zustand/middleware'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import { logger } from '@/lib/logger'

/**
 * App updates from https://github.com/vaultdbai/nandi/releases/latest/download/latest.json (see
 * `plugins.updater` in tauri.conf.json). The core checks the signature
 * against the bundled public key before installing anything.
 *
 * - `checkForUpdate` runs at startup and every `UPDATE_CHECK_INTERVAL_MS`
 * - When a newer version exists, the title bar shows an Update button
 * - `installUpdate` downloads, installs, and restarts the app
 */

/** How often a running app looks for a new version */
export const UPDATE_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000

export type UpdateStatus =
  'idle' | 'checking' | 'available' | 'downloading' | 'installing' | 'error'

interface UpdateState {
  status: UpdateStatus
  /** The newer version, while one is available */
  version: string | null
  notes: string | null
  /** 0–100 while downloading, when the size is known */
  progress: number | null
  error: string | null
}

const initialState: UpdateState = {
  status: 'idle',
  version: null,
  notes: null,
  progress: null,
  error: null,
}

export const useUpdateStore = create<UpdateState>()(
  devtools(() => initialState, { name: 'update-store' })
)

// The plugin's handle to the pending update (not serializable, so kept out
// of the store)
let pending: Update | null = null

const setState = (state: Partial<UpdateState>, action: string) =>
  useUpdateStore.setState(state, undefined, action)

/**
 * Looks for a newer version. Returns whether one is available. Errors (for
 * example, offline) are logged and leave the state as it was, unless
 * `reportErrors`.
 */
export async function checkForUpdate({
  reportErrors = false,
}: { reportErrors?: boolean } = {}): Promise<boolean> {
  const { status } = useUpdateStore.getState()
  if (status === 'downloading' || status === 'installing') return true
  setState({ status: 'checking', error: null }, 'update/checking')
  try {
    const update = await check()
    pending = update
    if (update) {
      logger.info(`Update available: ${update.version}`)
      setState(
        {
          status: 'available',
          version: update.version,
          notes: update.body ?? null,
        },
        'update/available'
      )
      return true
    }
    setState({ ...initialState }, 'update/none')
    return false
  } catch (error) {
    logger.warn('Update check failed', { error })
    // A failed check never hides an update already found
    setState({ status: pending ? 'available' : 'idle' }, 'update/checkFailed')
    if (reportErrors) throw error
    return pending !== null
  }
}

/** Downloads and installs the pending update, then restarts the app. */
export async function installUpdate(): Promise<void> {
  const update = pending
  if (!update) return
  let total = 0
  let received = 0
  setState({ status: 'downloading', progress: null }, 'update/downloading')
  try {
    await update.downloadAndInstall(event => {
      switch (event.event) {
        case 'Started':
          total = event.data.contentLength ?? 0
          break
        case 'Progress':
          received += event.data.chunkLength
          if (total > 0) {
            setState(
              { progress: Math.min(100, Math.round((received / total) * 100)) },
              'update/progress'
            )
          }
          break
        case 'Finished':
          setState({ status: 'installing', progress: 100 }, 'update/installing')
          break
      }
    })
    await relaunch()
  } catch (error) {
    logger.error('Update installation failed', { error })
    setState(
      { status: 'error', error: String(error), progress: null },
      'update/installFailed'
    )
  }
}

/** Checks now and then on an interval; returns a stop function. */
export function startUpdateChecks(): () => void {
  const first = setTimeout(() => void checkForUpdate(), 5_000)
  const interval = setInterval(
    () => void checkForUpdate(),
    UPDATE_CHECK_INTERVAL_MS
  )
  return () => {
    clearTimeout(first)
    clearInterval(interval)
  }
}

/** For tests */
export function resetUpdateStore(): void {
  pending = null
  useUpdateStore.setState(initialState)
}
