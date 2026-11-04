import type { PaletteItem } from './palette-items'

/** Recently opened palette items, per browser profile (a convenience only) */
const STORAGE_KEY = 'nandi.palette.recents'
export const MAX_RECENTS = 6

export function loadRecents(): PaletteItem[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY)
    if (!raw) return []
    const parsed: unknown = JSON.parse(raw)
    if (!Array.isArray(parsed)) return []
    return parsed
      .filter(
        (item): item is PaletteItem =>
          typeof item === 'object' &&
          item !== null &&
          typeof (item as PaletteItem).key === 'string' &&
          typeof (item as PaletteItem).name === 'string' &&
          typeof (item as PaletteItem).kind === 'string'
      )
      .slice(0, MAX_RECENTS)
  } catch {
    return []
  }
}

/** Moves `item` to the front; never throws */
export function addRecent(item: PaletteItem): void {
  try {
    const next = [
      item,
      ...loadRecents().filter(recent => recent.key !== item.key),
    ].slice(0, MAX_RECENTS)
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
  } catch {
    // Storage unavailable (private window, quota): recents are optional
  }
}

export function clearRecents(): void {
  try {
    localStorage.removeItem(STORAGE_KEY)
  } catch {
    // Ignore: see addRecent
  }
}
