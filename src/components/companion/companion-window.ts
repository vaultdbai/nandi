/**
 * Window operations for the companion webview. Each is a no-op outside
 * the desktop app (the companion page can also be opened in a browser).
 *
 * Handing off to the main window: `openInMain` shows and focuses the main
 * window and emits `companion://navigate` with the target view, which the
 * main window opens (`useMainWindowEventListeners`).
 */

import { emit, listen } from '@tauri-apps/api/event'
import { LogicalPosition, LogicalSize } from '@tauri-apps/api/dpi'
import {
  Window,
  currentMonitor,
  getCurrentWindow,
} from '@tauri-apps/api/window'
import { client, isDesktop } from '@/lib/client'
import { NAVIGATE_EVENT, OPEN_SETTINGS_EVENT } from '@/lib/companion-events'
import { logger } from '@/lib/logger'
import type { SettingsSection } from '@/lib/settings-nav'
import type { View } from '@/store/navigation-store'
import { applyBisonMotion } from '@/lib/bison-motion'
import { applyPalette, storedPalette } from '@/themes/palettes'

/** The companion's three states */
export type CompanionState = 'collapsed' | 'expanded' | 'overlay'

export const COLLAPSED_SIZE = { width: 192, height: 128 } as const
export const EXPANDED_SIZE = { width: 420, height: 640 } as const
/** The overlay spans the monitor's width at this height */
export const OVERLAY_HEIGHT = 560

/** Views the companion links to (main window `View`s) */
export type CompanionTarget = View

/**
 * Sizes the window for `state`. The bison and the card keep to the
 * window's screen corner (Rust's `place_companion`); the overlay spans the
 * monitor's width.
 */
export async function resizeCompanion(state: CompanionState): Promise<void> {
  if (!isDesktop()) return
  try {
    const window = getCurrentWindow()
    // Told first, so the overlay's own move isn't taken for a drag
    await client.placeCompanion(state)
    const monitor = state === 'overlay' ? await currentMonitor() : null
    if (monitor) {
      const scale = monitor.scaleFactor
      const width = monitor.size.width / scale
      const height = monitor.size.height / scale
      await window.setSize(new LogicalSize(width, OVERLAY_HEIGHT))
      await window.setPosition(
        new LogicalPosition(
          monitor.position.x / scale,
          monitor.position.y / scale + Math.max(0, height - OVERLAY_HEIGHT)
        )
      )
      return
    }
  } catch (error) {
    logger.warn('Companion: resize failed', { error })
  }
}

export async function hideCompanion(): Promise<void> {
  if (!isDesktop()) return
  try {
    await getCurrentWindow().hide()
  } catch (error) {
    logger.warn('Companion: hide failed', { error })
  }
}

/** Shows and focuses the main window, and asks it to open `target` */
export async function openInMain(target: CompanionTarget): Promise<void> {
  if (!isDesktop()) return
  try {
    const main = await Window.getByLabel('main')
    await main?.show()
    await main?.setFocus()
    await emit(NAVIGATE_EVENT, target)
  } catch (error) {
    logger.warn('Companion: could not open the main window', { error })
  }
}

/** Shows the main window on a Settings pane ("Add a model") */
export async function openSettingsInMain(pane: SettingsSection): Promise<void> {
  if (!isDesktop()) return
  try {
    const main = await Window.getByLabel('main')
    await main?.show()
    await main?.setFocus()
    await emit(OPEN_SETTINGS_EVENT, { pane })
  } catch (error) {
    logger.warn('Companion: could not open Settings', { error })
  }
}

/** Whether the companion window is on screen (hidden: notify instead) */
export async function companionVisible(): Promise<boolean> {
  if (!isDesktop()) return true
  try {
    return await getCurrentWindow().isVisible()
  } catch {
    return true
  }
}

/**
 * The bison's right-click menu, native so it isn't cut off by the small
 * window: open the card, open Nandi, hide the bison, quit
 */
export async function showBisonMenu(actions: {
  expand: () => void
  labels: { expand: string; openNandi: string; hide: string; quit: string }
}): Promise<void> {
  if (!isDesktop()) return
  try {
    const { Menu, MenuItem, PredefinedMenuItem } =
      await import('@tauri-apps/api/menu')
    const menu = await Menu.new({
      items: [
        await MenuItem.new({
          id: 'bison-expand',
          text: actions.labels.expand,
          action: actions.expand,
        }),
        await MenuItem.new({
          id: 'bison-open-nandi',
          text: actions.labels.openNandi,
          action: () => void openInMain({ kind: 'home' }),
        }),
        await PredefinedMenuItem.new({ item: 'Separator' }),
        await MenuItem.new({
          id: 'bison-hide',
          text: actions.labels.hide,
          action: () => void client.setCompanionBubble(false),
        }),
        await MenuItem.new({
          id: 'bison-quit',
          text: actions.labels.quit,
          action: () => void client.quitNandi(),
        }),
      ],
    })
    await menu.popup()
  } catch (error) {
    logger.warn('Companion: the bison menu failed', { error })
  }
}

/** Starts the window being dragged (from the header) */
export function startDragging(): void {
  if (!isDesktop()) return
  getCurrentWindow()
    .startDragging()
    .catch((error: unknown) => {
      logger.warn('Companion: drag failed', { error })
    })
}

/** The theme and palette the main window saved (`ThemeProvider`) */
export function applyTheme(): void {
  const saved = localStorage.getItem('ui-theme') || 'system'
  const dark =
    saved === 'system'
      ? window.matchMedia('(prefers-color-scheme: dark)').matches
      : saved === 'dark'
  const root = document.documentElement
  root.classList.remove('light', 'dark')
  root.classList.add(dark ? 'dark' : 'light')
  applyPalette(storedPalette(), root)
  applyBisonMotion(root)
}

/** Follows theme changes made in the main window. Returns a cleanup. */
export function followTheme(): () => void {
  applyTheme()
  if (!isDesktop()) return () => undefined
  const unlisten = listen('theme-changed', () => applyTheme())
  const unlistenFocus = getCurrentWindow().onFocusChanged(({ payload }) => {
    if (payload) applyTheme()
  })
  return () => {
    void unlisten.then(fn => fn())
    void unlistenFocus.then(fn => fn())
  }
}
