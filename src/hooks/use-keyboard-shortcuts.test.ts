import { describe, expect, it, beforeEach, vi } from 'vitest'
import { fireEvent, renderHook, waitFor } from '@testing-library/react'
import type { CommandContext } from '@/lib/commands/types'
import { useKeyboardShortcuts } from './use-keyboard-shortcuts'

const { executeCommand, isFullscreen } = vi.hoisted(() => ({
  executeCommand: vi.fn(),
  isFullscreen: vi.fn(),
}))

vi.mock('@/lib/commands/registry', () => ({ executeCommand }))
vi.mock('@/lib/commands/live-commands', () => ({
  REFRESH_COMMAND: 'refresh-from-disk',
}))
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

  it('F5 and Cmd/Ctrl+R refresh from disk instead of reloading', () => {
    renderHook(() => useKeyboardShortcuts(context))
    const f5 = new KeyboardEvent('keydown', { key: 'F5', cancelable: true })
    document.dispatchEvent(f5)
    fireEvent.keyDown(document, { key: 'r', ctrlKey: true })
    fireEvent.keyDown(document, { key: 'R', metaKey: true, shiftKey: true })
    expect(f5.defaultPrevented).toBe(true)
    expect(executeCommand).toHaveBeenCalledTimes(3)
    expect(executeCommand).toHaveBeenCalledWith('refresh-from-disk', context)
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
