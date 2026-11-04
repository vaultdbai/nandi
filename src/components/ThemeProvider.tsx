import { useEffect, useLayoutEffect, useState, useRef } from 'react'
import { emit } from '@tauri-apps/api/event'
import { useWindowIconEffect } from '@/hooks/useWindowIconEffect'
import { applyBisonMotion } from '@/lib/bison-motion'
import { isDesktop } from '@/lib/client'
import { ThemeProviderContext, type Theme } from '@/lib/theme-context'
import { usePreferences } from '@/services/preferences'
import {
  applyPalette,
  PALETTE_STORAGE_KEY,
  resolvePalette,
  storedPalette,
  type PaletteId,
} from '@/themes/palettes'

interface ThemeProviderProps {
  children: React.ReactNode
  defaultTheme?: Theme
  storageKey?: string
}

export function ThemeProvider({
  children,
  defaultTheme = 'system',
  storageKey = 'ui-theme',
  ...props
}: ThemeProviderProps) {
  const [theme, setTheme] = useState<Theme>(
    () => (localStorage.getItem(storageKey) as Theme) || defaultTheme
  )
  const [palette, setPalette] = useState<PaletteId>(storedPalette)

  // Load theme from persistent preferences
  const { data: preferences } = usePreferences()
  const hasSyncedPreferences = useRef(false)

  // Sync theme with preferences when they load
  // This is a legitimate case of syncing with external async state (persistent preferences)
  // The ref ensures this only happens once when preferences first load
  useLayoutEffect(() => {
    if (preferences?.theme && !hasSyncedPreferences.current) {
      hasSyncedPreferences.current = true
      setTheme(preferences.theme as Theme)
      const saved = resolvePalette(preferences.palette)
      localStorage.setItem(PALETTE_STORAGE_KEY, saved)
      setPalette(saved)
    }
  }, [preferences?.theme, preferences?.palette])

  useLayoutEffect(() => {
    applyPalette(palette)
  }, [palette])

  useWindowIconEffect(palette)

  // Whether the bison moves (Settings › Appearance)
  useEffect(() => applyBisonMotion(), [])

  useEffect(() => {
    const root = window.document.documentElement
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)')

    const applyTheme = (isDark: boolean) => {
      root.classList.remove('light', 'dark')
      root.classList.add(isDark ? 'dark' : 'light')
    }

    if (theme === 'system') {
      applyTheme(mediaQuery.matches)

      const handleChange = (e: MediaQueryListEvent) => applyTheme(e.matches)
      mediaQuery.addEventListener('change', handleChange)
      return () => mediaQuery.removeEventListener('change', handleChange)
    }

    applyTheme(theme === 'dark')
  }, [theme])

  const value = {
    theme,
    setTheme: (newTheme: Theme) => {
      localStorage.setItem(storageKey, newTheme)
      setTheme(newTheme)
      // Notify other windows (e.g., quick pane) of theme change
      if (isDesktop()) void emit('theme-changed', { theme: newTheme })
    },
    palette,
    setPalette: (newPalette: PaletteId) => {
      localStorage.setItem(PALETTE_STORAGE_KEY, newPalette)
      setPalette(newPalette)
      if (isDesktop()) void emit('theme-changed', { palette: newPalette })
    },
  }

  return (
    <ThemeProviderContext.Provider {...props} value={value}>
      {children}
    </ThemeProviderContext.Provider>
  )
}
