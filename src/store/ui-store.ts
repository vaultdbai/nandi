import { create } from 'zustand'
import { devtools } from 'zustand/middleware'
import type { SettingsSection } from '@/lib/settings-nav'

interface UIState {
  leftSidebarVisible: boolean
  /** The work tray on the right */
  rightSidebarVisible: boolean
  commandPaletteOpen: boolean

  toggleLeftSidebar: () => void
  setLeftSidebarVisible: (visible: boolean) => void
  toggleRightSidebar: () => void
  setRightSidebarVisible: (visible: boolean) => void
  toggleCommandPalette: () => void
  setCommandPaletteOpen: (open: boolean) => void
  /** Until when (epoch ms) the bull shows `done` */
  mascotDoneUntil: number
  /** The bull's `done` pose for a moment (an approval, the phrase) */
  celebrateMascot: (ms?: number) => void
  /** The Settings section last opened this session */
  settingsSection: SettingsSection | null
  setSettingsSection: (section: SettingsSection) => void
  setSquareCorners: (enabled: boolean) => void
}

export const useUIStore = create<UIState>()(
  devtools(
    set => ({
      leftSidebarVisible: true,
      rightSidebarVisible: true,
      commandPaletteOpen: false,
      settingsSection: null,
      mascotDoneUntil: 0,

      toggleLeftSidebar: () =>
        set(
          state => ({ leftSidebarVisible: !state.leftSidebarVisible }),
          undefined,
          'toggleLeftSidebar'
        ),

      setLeftSidebarVisible: visible =>
        set(
          { leftSidebarVisible: visible },
          undefined,
          'setLeftSidebarVisible'
        ),

      toggleRightSidebar: () =>
        set(
          state => ({ rightSidebarVisible: !state.rightSidebarVisible }),
          undefined,
          'toggleRightSidebar'
        ),

      setRightSidebarVisible: visible =>
        set(
          { rightSidebarVisible: visible },
          undefined,
          'setRightSidebarVisible'
        ),

      toggleCommandPalette: () =>
        set(
          state => ({ commandPaletteOpen: !state.commandPaletteOpen }),
          undefined,
          'toggleCommandPalette'
        ),

      setCommandPaletteOpen: open =>
        set({ commandPaletteOpen: open }, undefined, 'setCommandPaletteOpen'),

      celebrateMascot: (ms = 1500) =>
        set({ mascotDoneUntil: Date.now() + ms }, undefined, 'celebrateMascot'),

      setSettingsSection: section =>
        set({ settingsSection: section }, undefined, 'setSettingsSection'),

      setSquareCorners: (enabled: boolean) => {
        document.documentElement.classList.toggle('square-corners', enabled)
      },
    }),
    {
      name: 'ui-store',
    }
  )
)
