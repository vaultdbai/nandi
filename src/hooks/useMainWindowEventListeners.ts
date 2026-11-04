import { useEffect } from 'react'
import { listen } from '@tauri-apps/api/event'
import { isDesktop } from '@/lib/client'
import { useCommandContext } from './use-command-context'
import { useKeyboardShortcuts } from './use-keyboard-shortcuts'
import { parseView, useNavigationStore } from '@/store/navigation-store'
import { NAVIGATE_EVENT, OPEN_SETTINGS_EVENT } from '@/lib/companion-events'
import { logger } from '@/lib/logger'
import { isSettingsSection, openSettings } from '@/lib/settings-nav'

/**
 * Main window event listeners - handles global keyboard shortcuts and cross-window events.
 *
 * This hook composes specialized hooks for different event types:
 * - useKeyboardShortcuts: Global keyboard shortcuts (Cmd+, Cmd+1, Cmd+2)
 * - The companion window's hand-off: open a view here
 */
export function useMainWindowEventListeners() {
  const commandContext = useCommandContext()

  useKeyboardShortcuts(commandContext)

  // The companion window asks the main window to open a view
  useEffect(() => {
    if (!isDesktop()) return
    const unlisten = listen<unknown>(NAVIGATE_EVENT, event => {
      const view = parseView(event.payload)
      if (view) useNavigationStore.getState().navigate(view)
    })
    return () => {
      void unlisten
        .then(fn => fn())
        .catch((error: unknown) => {
          logger.warn('Failed to remove the companion navigate listener', {
            error,
          })
        })
    }
  }, [])

  // ... or a Settings pane ("Add a model" in the companion)
  useEffect(() => {
    if (!isDesktop()) return
    const unlisten = listen<{ pane?: unknown }>(OPEN_SETTINGS_EVENT, event => {
      const pane = event.payload.pane
      if (isSettingsSection(pane)) openSettings(pane)
    })
    return () => {
      void unlisten
        .then(fn => fn())
        .catch((error: unknown) => {
          logger.warn('Failed to remove the companion settings listener', {
            error,
          })
        })
    }
  }, [])
}
