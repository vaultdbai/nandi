import { beforeEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/test-utils'
import { check, type Update } from '@tauri-apps/plugin-updater'
import * as updates from '@/store/update-store'
import { UpdateButton } from './UpdateButton'

vi.mock('@tauri-apps/plugin-process', () => ({
  relaunch: vi.fn().mockResolvedValue(undefined),
}))

describe('UpdateButton', () => {
  beforeEach(() => {
    updates.resetUpdateStore()
  })

  it('is hidden until an update is available', () => {
    const { container } = render(<UpdateButton />)
    expect(container).toBeEmptyDOMElement()
  })

  it('installs with one click', async () => {
    const downloadAndInstall = vi.fn().mockResolvedValue(undefined)
    vi.mocked(check).mockResolvedValue({
      version: '0.2.0',
      downloadAndInstall,
    } as unknown as Update)
    await updates.checkForUpdate()
    render(<UpdateButton />)
    await userEvent.click(
      await screen.findByRole('button', { name: /0\.2\.0/ })
    )
    expect(downloadAndInstall).toHaveBeenCalled()
  })

  it('shows progress and is disabled while downloading', () => {
    updates.useUpdateStore.setState({
      status: 'downloading',
      version: '0.2.0',
      progress: 42,
    })
    render(<UpdateButton />)
    const button = screen.getByRole('button', { name: /42/ })
    expect(button).toBeDisabled()
  })

  it('offers a retry after a failed install', () => {
    updates.useUpdateStore.setState({
      status: 'error',
      version: '0.2.0',
      error: 'bad signature',
    })
    render(<UpdateButton />)
    expect(screen.getByRole('button', { name: /retry/i })).toBeEnabled()
  })
})
