/**
 * Where a row's links go. Home's rows open views in the main window's
 * navigation; the companion is its own webview, so it provides an opener
 * that hands views (and Settings panes) to the main window instead, and
 * keeps Request rows in its own conversation. Shared rows (Inbox, Running,
 * Recent) call `useViewOpener()` rather than navigating directly.
 */
import { createContext, useContext } from 'react'
import { openSettings, type SettingsSection } from '@/lib/settings-nav'
import { useNavigationStore, type View } from '@/store/navigation-store'

export interface ViewOpener {
  open: (view: View) => void
  openSettings: (section: SettingsSection) => void
}

const mainWindowOpener: ViewOpener = {
  open: view => useNavigationStore.getState().navigate(view),
  openSettings: section => openSettings(section),
}

export const ViewOpenerContext = createContext<ViewOpener>(mainWindowOpener)

export function useViewOpener(): ViewOpener {
  return useContext(ViewOpenerContext)
}
