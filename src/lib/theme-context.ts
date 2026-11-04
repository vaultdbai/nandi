import { createContext } from 'react'
import { DEFAULT_PALETTE, type PaletteId } from '@/themes/palettes'

export type Theme = 'dark' | 'light' | 'system'

export interface ThemeProviderState {
  /** Appearance: light, dark, or follow the system */
  theme: Theme
  setTheme: (theme: Theme) => void
  /** Colour palette, independent of the appearance */
  palette: PaletteId
  setPalette: (palette: PaletteId) => void
}

const initialState: ThemeProviderState = {
  theme: 'system',
  setTheme: () => null,
  palette: DEFAULT_PALETTE,
  setPalette: () => null,
}

export const ThemeProviderContext =
  createContext<ThemeProviderState>(initialState)
