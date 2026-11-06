import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { ImageStatus } from '@/lib/bindings'
import { emitMockEvent } from '@/test/mock-events'
import { SandboxBuildPanel, SandboxImagePanel } from './SandboxImagePanel'

const MB = 1024 * 1024

const status = (overrides: Partial<ImageStatus> = {}): ImageStatus => ({
  tag: 'ghcr.io/vaultdbai/nandi-harness:sandbox-6',
  present: false,
  current: false,
  dockerRunning: true,
  building: false,
  pulling: false,
  downloadBytes: 450 * MB,
  progress: null,
  builtBy: null,
  log: [],
  error: null,
  ...overrides,
})

const mock = (data: ImageStatus) =>
  Object.assign(commands, {
    getImageStatus: vi.fn().mockResolvedValue({ status: 'ok', data }),
    buildImage: vi.fn().mockResolvedValue({ status: 'ok', data: null }),
    pullImage: vi.fn().mockResolvedValue({ status: 'ok', data: null }),
  })

describe('SandboxImagePanel', () => {
  it('says a missing image downloads on first use, with its size', async () => {
    mock(status())
    render(<SandboxImagePanel image="harness" />)
    expect(await screen.findByTestId('image-state')).toHaveTextContent(
      'Not on this computer yet. Nandi downloads it (450 MB) the first time an agent needs it.'
    )
    expect(
      screen.getByRole('button', { name: 'Download now (450 MB)' })
    ).toBeEnabled()
    // Building is never offered here
    expect(screen.queryByRole('button', { name: /Build/ })).toBeNull()
  })

  it('downloads ahead of time and shows the bytes as they come', async () => {
    mock(status())
    const user = userEvent.setup()
    render(<SandboxImagePanel image="harness" />)
    await user.click(
      await screen.findByRole('button', { name: 'Download now (450 MB)' })
    )
    expect(commands.pullImage).toHaveBeenCalledWith('harness')
    // What the core now reports
    mock(
      status({
        pulling: true,
        progress: { doneBytes: 90 * MB, totalBytes: 450 * MB },
        log: ['a1b2: Pulling fs layer'],
      })
    )
    act(() =>
      emitMockEvent('images://pull', {
        image: 'harness',
        build: false,
        line: null,
        progress: { doneBytes: 90 * MB, totalBytes: 450 * MB },
        finished: null,
        error: null,
      })
    )
    expect(await screen.findByRole('progressbar')).toHaveAttribute(
      'aria-valuenow',
      '20'
    )
    expect(screen.getByTestId('image-state')).toHaveTextContent(
      'Downloading: 90 MB of 450 MB (20%).'
    )
    expect(screen.getByRole('button', { name: /Downloading/ })).toBeDisabled()
  })

  it('offers Try again after a failed download, never a build', async () => {
    mock(status({ error: "Couldn't download it (no such host)" }))
    const user = userEvent.setup()
    render(<SandboxImagePanel image="runtime" />)
    expect(await screen.findByTestId('image-state')).toHaveTextContent(
      'no such host'
    )
    await user.click(screen.getByRole('button', { name: 'Try again' }))
    expect(commands.pullImage).toHaveBeenCalledWith('runtime')
    expect(screen.queryByRole('button', { name: /Build/ })).toBeNull()
  })

  it('is ready with no button once the current image is here', async () => {
    mock(status({ present: true, current: true }))
    render(<SandboxImagePanel image="harness" />)
    expect(await screen.findByTestId('image-state')).toHaveTextContent(
      'Ready (ghcr.io/vaultdbai/nandi-harness:sandbox-6).'
    )
    expect(screen.queryByRole('button')).toBeNull()
  })

  it('says an older image is replaced the next time an agent needs it', async () => {
    mock(status({ present: true, current: false, builtBy: '0.8.0' }))
    render(<SandboxImagePanel image="runtime" />)
    expect(await screen.findByTestId('image-state')).toHaveTextContent(
      'From an older Nandi (ghcr.io/vaultdbai/nandi-harness:sandbox-6). Nandi downloads the current one the next time an agent needs it.'
    )
  })

  it('says when Docker isn’t running, and can’t download then', async () => {
    mock(status({ dockerRunning: false }))
    render(<SandboxImagePanel image="runtime" />)
    await waitFor(() =>
      expect(screen.getByTestId('image-state')).toHaveTextContent(
        /Docker isn.t running/
      )
    )
    expect(screen.getByRole('button', { name: /Download now/ })).toBeDisabled()
  })
})

describe('SandboxBuildPanel', () => {
  it('builds an image here only when asked, as an option', async () => {
    mock(status())
    const user = userEvent.setup()
    render(<SandboxBuildPanel />)
    await user.click(
      await screen.findByRole('button', {
        name: "Build Coding agents' sandbox here",
      })
    )
    expect(commands.buildImage).toHaveBeenCalledWith('harness')
    expect(commands.pullImage).not.toHaveBeenCalled()
  })
})
