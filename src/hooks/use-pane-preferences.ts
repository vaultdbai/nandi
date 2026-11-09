/**
 * Both panes' visibility, kept in this machine's preferences. Applied once
 * the preferences load; every show or hide after that is saved. Until the
 * user shows or hides the work tray, it follows the window: hidden below
 * `NARROW_WINDOW_PX`.
 */
import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { paneLayout } from '@/components/layout/layout-model'
import type { AppPreferences } from '@/lib/tauri-bindings'
import {
  preferencesQueryKeys,
  savePaneLayout,
  usePreferences,
} from '@/services/preferences'
import { useUIStore } from '@/store/ui-store'

export function usePanePreferences() {
  const queryClient = useQueryClient()
  const loaded = usePreferences().data !== undefined

  useEffect(() => {
    if (!loaded) return
    const saved = queryClient.getQueryData<AppPreferences>(
      preferencesQueryKeys.preferences()
    )
    let trayPicked = saved?.workTrayVisible ?? null
    // Our own changes aren't the user's: never saved
    let applying = false
    const apply = (state: Partial<ReturnType<typeof useUIStore.getState>>) => {
      applying = true
      useUIStore.setState(state)
      applying = false
    }

    const initial = paneLayout(saved, window.innerWidth)
    apply({
      leftSidebarVisible: initial.sidebar,
      rightSidebarVisible: initial.tray,
    })

    const followWidth = () => {
      if (trayPicked !== null) return
      apply({
        rightSidebarVisible: paneLayout(
          { workTrayVisible: null },
          window.innerWidth
        ).tray,
      })
    }

    const unsubscribe = useUIStore.subscribe((state, previous) => {
      if (applying) return
      const sidebar = state.leftSidebarVisible !== previous.leftSidebarVisible
      const tray = state.rightSidebarVisible !== previous.rightSidebarVisible
      if (!sidebar && !tray) return
      if (tray) trayPicked = state.rightSidebarVisible
      void savePaneLayout(queryClient, {
        sidebarVisible: state.leftSidebarVisible,
        workTrayVisible: trayPicked,
      })
    })

    window.addEventListener('resize', followWidth)
    return () => {
      unsubscribe()
      window.removeEventListener('resize', followWidth)
    }
  }, [loaded, queryClient])
}
