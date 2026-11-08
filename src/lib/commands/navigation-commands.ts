import { Sidebar, Settings } from 'lucide-react'
import { openSettings, SETTINGS_SECTIONS } from '@/lib/settings-nav'
import { useUIStore } from '@/store/ui-store'
import type { AppCommand } from './types'

export const navigationCommands: AppCommand[] = [
  {
    id: 'show-left-sidebar',
    labelKey: 'commands.showLeftSidebar.label',
    descriptionKey: 'commands.showLeftSidebar.description',
    icon: Sidebar,
    group: 'navigation',
    shortcut: '⌘+1',
    keywords: ['sidebar', 'left', 'panel', 'show'],

    execute: () => {
      useUIStore.getState().setLeftSidebarVisible(true)
    },

    isAvailable: () => !useUIStore.getState().leftSidebarVisible,
  },

  {
    id: 'hide-left-sidebar',
    labelKey: 'commands.hideLeftSidebar.label',
    descriptionKey: 'commands.hideLeftSidebar.description',
    icon: Sidebar,
    group: 'navigation',
    shortcut: '⌘+1',
    keywords: ['sidebar', 'left', 'panel', 'hide'],

    execute: () => {
      useUIStore.getState().setLeftSidebarVisible(false)
    },

    isAvailable: () => useUIStore.getState().leftSidebarVisible,
  },

  {
    id: 'show-right-sidebar',
    labelKey: 'commands.showRightSidebar.label',
    descriptionKey: 'commands.showRightSidebar.description',
    icon: Sidebar,
    group: 'navigation',
    shortcut: '⌘+2',
    keywords: ['tray', 'work', 'right', 'panel', 'show'],

    execute: () => {
      useUIStore.getState().setRightSidebarVisible(true)
    },

    isAvailable: () => !useUIStore.getState().rightSidebarVisible,
  },

  {
    id: 'hide-right-sidebar',
    labelKey: 'commands.hideRightSidebar.label',
    descriptionKey: 'commands.hideRightSidebar.description',
    icon: Sidebar,
    group: 'navigation',
    shortcut: '⌘+2',
    keywords: ['tray', 'work', 'right', 'panel', 'hide'],

    execute: () => {
      useUIStore.getState().setRightSidebarVisible(false)
    },

    isAvailable: () => useUIStore.getState().rightSidebarVisible,
  },

  {
    id: 'open-preferences',
    labelKey: 'commands.openPreferences.label',
    descriptionKey: 'commands.openPreferences.description',
    icon: Settings,
    group: 'settings',
    shortcut: '⌘+,',
    keywords: ['preferences', 'settings', 'config', 'options'],

    execute: context => {
      context.openPreferences()
    },
  },
  // "Settings: Models", "Settings: Logs", …
  ...SETTINGS_SECTIONS.map((section): AppCommand => ({
    id: `open-settings-${section}`,
    labelKey: `commands.openSettings.${section}`,
    icon: Settings,
    group: 'settings',
    keywords: ['settings', section],
    execute: () => openSettings(section),
  })),
]
