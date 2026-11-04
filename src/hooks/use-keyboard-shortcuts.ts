import { useEffect } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { useUIStore } from '@/store/ui-store'
import { executeCommand } from '@/lib/commands/registry'
import type { CommandContext } from '@/lib/commands/types'
import { isDesktop } from '@/lib/client'

/**
 * Handles global keyboard shortcuts for the application.
 *
 * Currently handles:
 * - Cmd/Ctrl+K : Toggle the command palette (works from inside inputs)
 * - Cmd/Ctrl+, : Open preferences
 * - Cmd/Ctrl+1 : Toggle left sidebar
 * - Cmd/Ctrl+W : Close the window (desktop)
 * - Cmd/Ctrl+M : Minimize the window (desktop)
 * - F11        : Enter or leave fullscreen (desktop)
 */
export function useKeyboardShortcuts(commandContext: CommandContext) {
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'F11') {
        e.preventDefault()
        void toggleFullscreen(commandContext)
        return
      }
      if (e.metaKey || e.ctrlKey) {
        switch (e.key.toLowerCase()) {
          case 'k': {
            if (e.shiftKey || e.altKey) break
            e.preventDefault()
            useUIStore.getState().toggleCommandPalette()
            break
          }
          case ',': {
            e.preventDefault()
            commandContext.openPreferences()
            break
          }
          case '1': {
            e.preventDefault()
            const { leftSidebarVisible, setLeftSidebarVisible } =
              useUIStore.getState()
            setLeftSidebarVisible(!leftSidebarVisible)
            break
          }
          case 'w': {
            if (e.shiftKey || e.altKey) break
            e.preventDefault()
            void executeCommand('window-close', commandContext)
            break
          }
          case 'm': {
            if (e.shiftKey || e.altKey) break
            e.preventDefault()
            void executeCommand('window-minimize', commandContext)
            break
          }
        }
      }
    }

    document.addEventListener('keydown', handleKeyDown)
    return () => document.removeEventListener('keydown', handleKeyDown)
  }, [commandContext])
}

async function toggleFullscreen(context: CommandContext): Promise<void> {
  if (!isDesktop()) return
  const fullscreen = await getCurrentWindow().isFullscreen()
  await executeCommand(
    fullscreen ? 'window-exit-fullscreen' : 'window-fullscreen',
    context
  )
}
