import { useEffect } from 'react'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { isDesktop } from '@/lib/client'
import { logger } from '@/lib/logger'
import { PALETTE_ICONS } from '@/themes/palette-icons'
import type { PaletteId } from '@/themes/palettes'

/**
 * The main window's icon (taskbar, Alt+Tab) follows the palette. The
 * installer, Start menu and macOS Dock keep the bundled nandi icon.
 */
export function useWindowIconEffect(palette: PaletteId) {
  useEffect(() => {
    if (!isDesktop()) return

    let cancelled = false
    const apply = async () => {
      // Only the main window has the permission; the companion keeps its own
      const window = getCurrentWindow()
      if (window.label !== 'main') return
      const response = await fetch(PALETTE_ICONS[palette])
      const bytes = new Uint8Array(await response.arrayBuffer())
      if (!cancelled) await window.setIcon(bytes)
    }
    apply().catch((error: unknown) =>
      logger.warn('Could not set the window icon', { error })
    )
    return () => {
      cancelled = true
    }
  }, [palette])
}
