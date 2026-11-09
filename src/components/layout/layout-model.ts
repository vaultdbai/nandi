/**
 * Which panes show: this machine's preferences, and the window width for a
 * work tray the user hasn't shown or hidden yet. Pure.
 */
import type { AppPreferences } from '@/lib/tauri-bindings'

/** Below this width the work tray starts hidden, until the user picks */
export const NARROW_WINDOW_PX = 1000

export interface PaneLayout {
  sidebar: boolean
  tray: boolean
}

export function paneLayout(
  preferences:
    Pick<AppPreferences, 'sidebarVisible' | 'workTrayVisible'> | undefined,
  width: number
): PaneLayout {
  return {
    sidebar: preferences?.sidebarVisible ?? true,
    tray: preferences?.workTrayVisible ?? width >= NARROW_WINDOW_PX,
  }
}
