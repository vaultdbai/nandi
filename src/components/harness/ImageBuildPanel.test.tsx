import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, waitFor } from '@/test/test-utils'
import { commands } from '@/lib/tauri-bindings'
import type { ImageStatus } from '@/lib/bindings'
import { emitMockEvent } from '@/test/mock-events'
import { ImageBuildPanel } from './ImageBuildPanel'

const status = (overrides: Partial<ImageStatus> = {}): ImageStatus => ({
  tag: 'ghcr.io/vaultdbai/nandi-harness:sandbox-5',
  present: false,
  current: false,
  dockerRunning: true,
  building: false,
  pulling: false,
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

describe('ImageBuildPanel', () => {
  it('downloads a missing image and shows its progress as it comes', async () => {
    mock(status())
    const user = userEvent.setup()
    render(<ImageBuildPanel image="harness" />)
    expect(await screen.findByTestId('image-state')).toHaveTextContent(
      'Not on this computer yet (ghcr.io/vaultdbai/nandi-harness:sandbox-5).'
    )
    await user.click(screen.getByRole('button', { name: 'Download' }))
    expect(commands.pullImage).toHaveBeenCalledWith('harness')
    // The core now reports the download and its lines
    mock(status({ pulling: true, log: ['a1b2: Pull complete'] }))
    act(() =>
      emitMockEvent('images://build', {
        image: 'harness',
        line: 'a1b2: Pull complete',
        finished: null,
        error: null,
      })
    )
    expect(await screen.findByText(/a1b2: Pull complete/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Downloading/ })).toBeDisabled()
    expect(screen.queryByRole('button', { name: 'Build here' })).toBeNull()
  })

  it('still builds here, for a computer that can’t reach the registry', async () => {
    mock(status({ error: "Couldn't download it (no such host)" }))
    const user = userEvent.setup()
    render(<ImageBuildPanel image="harness" />)
    expect(await screen.findByTestId('image-state')).toHaveTextContent(
      'no such host'
    )
    await user.click(screen.getByRole('button', { name: 'Build here' }))
    expect(commands.buildImage).toHaveBeenCalledWith('harness')
    mock(status({ building: true, log: ['Step 1/9 : FROM node'] }))
    act(() =>
      emitMockEvent('images://build', {
        image: 'harness',
        line: 'Step 1/9 : FROM node',
        finished: null,
        error: null,
      })
    )
    expect(await screen.findByText(/Step 1\/9 : FROM node/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /Building/ })).toBeDisabled()
  })

  it('offers Rebuild here once ready, and says which version built it', async () => {
    mock(status({ present: true, current: true, builtBy: '0.9.0' }))
    render(<ImageBuildPanel image="harness" />)
    expect(await screen.findByTestId('image-state')).toHaveTextContent(
      'Ready (ghcr.io/vaultdbai/nandi-harness:sandbox-5, built by Nandi 0.9.0).'
    )
    expect(screen.getByRole('button', { name: 'Rebuild here' })).toBeEnabled()
    expect(screen.queryByRole('button', { name: 'Download' })).toBeNull()
  })

  it('offers the current image for one from an older sandbox', async () => {
    mock(status({ present: true, current: false, builtBy: '0.8.0' }))
    render(<ImageBuildPanel image="runtime" />)
    expect(await screen.findByTestId('image-state')).toHaveTextContent(
      'From an older Nandi (ghcr.io/vaultdbai/nandi-harness:sandbox-5). Download the current one, or rebuild it here.'
    )
    expect(screen.getByRole('button', { name: 'Download' })).toBeEnabled()
    expect(screen.getByRole('button', { name: 'Build here' })).toBeEnabled()
  })

  it('says when Docker isn’t running, and can’t download or build then', async () => {
    mock(status({ dockerRunning: false }))
    render(<ImageBuildPanel image="runtime" />)
    await waitFor(() =>
      expect(screen.getByTestId('image-state')).toHaveTextContent(
        /Docker isn.t running/
      )
    )
    expect(screen.getByRole('button', { name: 'Download' })).toBeDisabled()
    expect(screen.getByRole('button', { name: 'Build here' })).toBeDisabled()
  })
})
