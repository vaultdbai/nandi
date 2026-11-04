/**
 * Whether the bison moves (`bison-pose.css`, `companion-bison.css`):
 * always (the default), when the system allows it (still under "reduce
 * motion", which Windows sets when Animation effects is off), or never. Kept on
 * this computer (localStorage, shared with the companion window, which
 * hears changes as `theme-changed`) and set on <html> as
 * `data-bison-motion`.
 */
import { emit } from '@tauri-apps/api/event'
import { isDesktop } from '@/lib/client'
import { logger } from '@/lib/logger'

export const BISON_MOTIONS = ['system', 'always', 'never'] as const
export type BisonMotion = (typeof BISON_MOTIONS)[number]

const STORAGE_KEY = 'bison-motion'

export function bisonMotion(): BisonMotion {
  try {
    const saved = localStorage.getItem(STORAGE_KEY)
    return BISON_MOTIONS.find(m => m === saved) ?? 'always'
  } catch {
    return 'always'
  }
}

export function applyBisonMotion(root: HTMLElement = document.documentElement) {
  root.dataset.bisonMotion = bisonMotion()
}

export function setBisonMotion(motion: BisonMotion) {
  try {
    localStorage.setItem(STORAGE_KEY, motion)
  } catch {
    // Blocked storage: it still applies until the app restarts
  }
  // Set directly: with storage blocked, reading it back gives the default
  document.documentElement.dataset.bisonMotion = motion
  if (isDesktop()) {
    emit('theme-changed', { bisonMotion: motion }).catch((error: unknown) =>
      logger.warn('Could not tell the companion about the bison', { error })
    )
  }
}
