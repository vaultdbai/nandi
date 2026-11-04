import { beforeEach, describe, expect, it, vi } from 'vitest'
import { check, type Update } from '@tauri-apps/plugin-updater'
import { relaunch } from '@tauri-apps/plugin-process'
import {
  checkForUpdate,
  installUpdate,
  resetUpdateStore,
  useUpdateStore,
} from './update-store'

vi.mock('@tauri-apps/plugin-process', () => ({
  relaunch: vi.fn().mockResolvedValue(undefined),
}))

type DownloadEvent = Parameters<
  Parameters<Update['downloadAndInstall']>[0] & object
>[0]

function fakeUpdate(
  run: (emit: (event: DownloadEvent) => void) => Promise<void> = async emit => {
    emit({ event: 'Started', data: { contentLength: 200 } })
    emit({ event: 'Progress', data: { chunkLength: 50 } })
    emit({ event: 'Progress', data: { chunkLength: 150 } })
    emit({ event: 'Finished' })
  }
): Update {
  return {
    version: '0.2.0',
    body: 'Faster delegation',
    downloadAndInstall: vi.fn(async (onEvent?: (e: DownloadEvent) => void) =>
      run(event => onEvent?.(event))
    ),
  } as unknown as Update
}

describe('update store', () => {
  beforeEach(() => {
    resetUpdateStore()
    vi.mocked(check).mockReset()
    vi.mocked(relaunch).mockClear()
  })

  it('records a newer version', async () => {
    vi.mocked(check).mockResolvedValue(fakeUpdate())
    await expect(checkForUpdate()).resolves.toBe(true)
    const state = useUpdateStore.getState()
    expect(state.status).toBe('available')
    expect(state.version).toBe('0.2.0')
    expect(state.notes).toBe('Faster delegation')
  })

  it('stays idle when already up to date', async () => {
    vi.mocked(check).mockResolvedValue(null)
    await expect(checkForUpdate()).resolves.toBe(false)
    expect(useUpdateStore.getState()).toMatchObject({
      status: 'idle',
      version: null,
    })
  })

  it('ignores failed background checks but reports them on request', async () => {
    vi.mocked(check).mockRejectedValue(new Error('offline'))
    await expect(checkForUpdate()).resolves.toBe(false)
    expect(useUpdateStore.getState().status).toBe('idle')
    await expect(checkForUpdate({ reportErrors: true })).rejects.toThrow(
      'offline'
    )
  })

  it('keeps a found update when a later check fails', async () => {
    vi.mocked(check).mockResolvedValueOnce(fakeUpdate())
    await checkForUpdate()
    vi.mocked(check).mockRejectedValueOnce(new Error('offline'))
    await expect(checkForUpdate()).resolves.toBe(true)
    expect(useUpdateStore.getState()).toMatchObject({
      status: 'available',
      version: '0.2.0',
    })
  })

  it('downloads, installs, and restarts', async () => {
    const update = fakeUpdate()
    vi.mocked(check).mockResolvedValue(update)
    await checkForUpdate()
    await installUpdate()
    expect(update.downloadAndInstall).toHaveBeenCalled()
    expect(useUpdateStore.getState()).toMatchObject({
      status: 'installing',
      progress: 100,
    })
    expect(relaunch).toHaveBeenCalled()
  })

  it('shows the error and keeps the version when installing fails', async () => {
    vi.mocked(check).mockResolvedValue(
      fakeUpdate(() => Promise.reject(new Error('bad signature')))
    )
    await checkForUpdate()
    await installUpdate()
    expect(useUpdateStore.getState()).toMatchObject({
      status: 'error',
      version: '0.2.0',
    })
    expect(useUpdateStore.getState().error).toContain('bad signature')
    expect(relaunch).not.toHaveBeenCalled()
  })
})
