/**
 * Colour palettes. Each one is a brand hue + chroma; the CSS in
 * `theme-variables.css` derives every surface from them, in both light and
 * dark appearance. Keep this list and the `[data-theme]` blocks in sync
 * (`palettes.test.ts` checks it).
 */

export const PALETTES = [
  { id: 'nandi', hue: 262, chroma: 0.2 },
  { id: 'aurora', hue: 300, chroma: 0.22 },
  { id: 'ocean', hue: 215, chroma: 0.14 },
  { id: 'sunset', hue: 32, chroma: 0.19 },
  { id: 'forest', hue: 158, chroma: 0.13 },
  { id: 'graphite', hue: 260, chroma: 0 },
  // Darker text and borders, deeper status colours (extra CSS block)
  { id: 'high-contrast', hue: 262, chroma: 0.2 },
] as const

export type PaletteId = (typeof PALETTES)[number]['id']

export const DEFAULT_PALETTE: PaletteId = 'nandi'

/** localStorage key shared by every window (main, companion, quick pane) */
export const PALETTE_STORAGE_KEY = 'ui-palette'

export function isPaletteId(value: unknown): value is PaletteId {
  return PALETTES.some(palette => palette.id === value)
}

/** `value` if it names a palette, else the default */
export function resolvePalette(value: unknown): PaletteId {
  return isPaletteId(value) ? value : DEFAULT_PALETTE
}

/** Sets `data-theme` on `<html>`; the default palette needs no attribute */
export function applyPalette(
  palette: PaletteId,
  root: HTMLElement = document.documentElement
): void {
  if (palette === DEFAULT_PALETTE) root.removeAttribute('data-theme')
  else root.setAttribute('data-theme', palette)
}

/** The palette the main window saved, for windows without preferences */
export function storedPalette(): PaletteId {
  try {
    return resolvePalette(localStorage.getItem(PALETTE_STORAGE_KEY))
  } catch {
    return DEFAULT_PALETTE
  }
}
