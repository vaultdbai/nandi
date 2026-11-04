/**
 * The Nandi app icon for each palette (`src/assets/brand/icons`, a
 * brand asset, see NOTICE). The bull inside the app and the
 * window icon both follow the palette the user picked. The mascot's
 * poses (png/mascot) are white on transparent, so one set fits them all.
 */
import carry from '@/assets/brand/mascot/nandi-bull-carry.png'
import done from '@/assets/brand/mascot/nandi-bull-done.png'
import rest from '@/assets/brand/mascot/nandi-bull-rest.png'
import wait from '@/assets/brand/mascot/nandi-bull-wait.png'
import aurora from '@/assets/brand/icons/nandi-icon-aurora.png'
import forest from '@/assets/brand/icons/nandi-icon-forest.png'
import graphite from '@/assets/brand/icons/nandi-icon-graphite.png'
import highContrast from '@/assets/brand/icons/nandi-icon-high-contrast.png'
import nandi from '@/assets/brand/icons/nandi-icon-nandi.png'
import ocean from '@/assets/brand/icons/nandi-icon-ocean.png'
import sunset from '@/assets/brand/icons/nandi-icon-sunset.png'
import type { NandiState } from '@/assets/brand/NandiBull'
import type { PaletteId } from './palettes'

export const PALETTE_ICONS: Record<PaletteId, string> = {
  nandi,
  aurora,
  ocean,
  sunset,
  forest,
  graphite,
  'high-contrast': highContrast,
}

export const MASCOT_POSES: Record<NandiState, string> = {
  rest,
  wait,
  carry,
  done,
}
