import { describe, expect, it, beforeEach, vi } from 'vitest'
import { fireEvent, renderHook, waitFor } from '@testing-library/react'
import type { CommandContext } from '@/lib/commands/types'
import { useKeyboardShortcuts } from './use-keyboard-shortcuts'

const { executeCommand, isFullscreen } = vi.hoisted(() => ({
  executeCommand: vi.fn(),
  isFullscreen: vi.fn(),
}))

vi.mock('@/lib/commands/registry', () => ({ executeCommand }))
vi.mock('@/lib/client', () => ({ isDesktop: () => true }))
vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ isFullscreen }),
}))

const context: CommandContext = {
  openPreferences: vi.fn(),
  showToast: vi.fn(),
}

describe('useKeyboardShortcuts window keys', () => {
  beforeEach(() => {
    executeCommand.mockReset()
    isFullscreen.mockReset()
  })

  it.each([
    ['w', 'window-close'],
    ['m', 'window-minimize'],
  ])('Cmd/Ctrl+%s runs %s', (key, commandId) => {
    renderHook(() => useKeyboardShortcuts(context))
    fireEvent.keyDown(document, { key, ctrlKey: true })
    fireEvent.keyDown(document, { key, metaKey: true })
    expect(executeCommand).toHaveBeenCalledTimes(2)
    expect(executeCommand).toHaveBeenCalledWith(commandId, context)
  })

  it('leaves Cmd/Ctrl+Shift+W alone', () => {
    renderHook(() => useKeyboardShortcuts(context))
    fireEvent.keyDown(document, { key: 'w', ctrlKey: true, shiftKey: true })
    expect(executeCommand).not.toHaveBeenCalled()
  })

  it.each([
    [false, 'window-fullscreen'],
    [true, 'window-exit-fullscreen'],
  ])('F11 when fullscreen is %s runs %s', async (fullscreen, commandId) => {
    isFullscreen.mockResolvedValue(fullscreen)
    renderHook(() => useKeyboardShortcuts(context))
    fireEvent.keyDown(document, { key: 'F11' })
    await waitFor(() =>
      expect(executeCommand).toHaveBeenCalledWith(commandId, context)
    )
  })
})
