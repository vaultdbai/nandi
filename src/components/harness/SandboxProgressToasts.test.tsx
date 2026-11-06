import { beforeEach, describe, expect, it, vi } from 'vitest'
import { toast } from 'sonner'
import { act, render } from '@/test/test-utils'
import { emitMockEvent } from '@/test/mock-events'
import { SandboxProgressToasts } from './SandboxProgressToasts'

vi.mock('sonner', () => ({
  toast: { loading: vi.fn(), success: vi.fn(), error: vi.fn() },
}))

const MB = 1024 * 1024
const pull = (payload: Record<string, unknown>) =>
  emitMockEvent('images://pull', {
    image: 'harness',
    build: false,
    line: null,
    progress: null,
    finished: null,
    error: null,
    ...payload,
  })

describe('SandboxProgressToasts', () => {
  beforeEach(() => vi.clearAllMocks())

  it('shows a download Nandi started by itself, in one toast', () => {
    render(<SandboxProgressToasts />)
    act(() => pull({ progress: { doneBytes: 45 * MB, totalBytes: 450 * MB } }))
    expect(toast.loading).toHaveBeenLastCalledWith(
      "Downloading: Coding agents' sandbox (10% of 450 MB)…",
      { id: 'image-harness' }
    )
    act(() => pull({ finished: true }))
    expect(toast.success).toHaveBeenCalledWith(
      "Ready: Coding agents' sandbox",
      { id: 'image-harness' }
    )
  })

  it('ends a failed download with Try again', () => {
    render(<SandboxProgressToasts />)
    act(() => pull({ finished: false, error: 'offline' }))
    expect(toast.error).toHaveBeenCalledWith(
      "Couldn't download: Coding agents' sandbox",
      expect.objectContaining({
        id: 'image-harness',
        description: 'offline',
        action: expect.objectContaining({ label: 'Try again' }),
      })
    )
  })

  it('leaves the advanced build to Settings', () => {
    render(<SandboxProgressToasts />)
    act(() => pull({ build: true, line: 'Step 1/9' }))
    expect(toast.loading).not.toHaveBeenCalled()
  })

  it('shows Claude Code’s one-time install with its size', () => {
    render(<SandboxProgressToasts />)
    act(() =>
      emitMockEvent('images://tools', {
        bundle: 'claudeCode',
        downloadBytes: 95 * MB,
        line: 'Installing claude-code from its lockfile…',
        finished: null,
        error: null,
      })
    )
    expect(toast.loading).toHaveBeenCalledWith(
      'Getting Claude Code (one time, about 95 MB)…',
      { id: 'tools-claudeCode' }
    )
  })
})
