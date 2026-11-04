import { afterEach, describe, expect, it } from 'vitest'
import css from '../theme-variables.css?raw'
import {
  applyPalette,
  DEFAULT_PALETTE,
  PALETTE_STORAGE_KEY,
  PALETTES,
  resolvePalette,
  storedPalette,
} from './palettes'

/** `[data-theme='id'] { --brand-h: H; --brand-c: C; }` blocks in the CSS */
function cssPalettes(): Map<string, { hue: number; chroma: number }> {
  const blocks = new Map<string, { hue: number; chroma: number }>()
  const pattern =
    /:root\[data-theme='([a-z0-9-]+)'\]\s*\{\s*--brand-h:\s*([\d.]+);\s*--brand-c:\s*([\d.]+);\s*\}/g
  for (const [, id, hue, chroma] of css.matchAll(pattern)) {
    blocks.set(id ?? '', { hue: Number(hue), chroma: Number(chroma) })
  }
  return blocks
}

describe('palettes', () => {
  afterEach(() => {
    document.documentElement.removeAttribute('data-theme')
    localStorage.clear()
  })

  it('matches the CSS: one [data-theme] block per non-default palette', () => {
    const blocks = cssPalettes()
    const others = PALETTES.filter(p => p.id !== DEFAULT_PALETTE)
    expect([...blocks.keys()].sort()).toEqual(others.map(p => p.id).sort())
    for (const palette of others) {
      expect(blocks.get(palette.id)).toEqual({
        hue: palette.hue,
        chroma: palette.chroma,
      })
    }
  })

  it('the default palette is the :root brand', () => {
    const nandi = PALETTES.find(p => p.id === DEFAULT_PALETTE)
    expect(css).toContain(`--brand-h: ${nandi?.hue};`)
    expect(css).toContain(`--brand-c: ${nandi?.chroma};`)
  })

  it('falls back to the default for unknown values', () => {
    expect(resolvePalette('aurora')).toBe('aurora')
    expect(resolvePalette('neon')).toBe(DEFAULT_PALETTE)
    expect(resolvePalette(null)).toBe(DEFAULT_PALETTE)
  })

  it('sets data-theme, and clears it for the default', () => {
    const root = document.documentElement
    applyPalette('sunset')
    expect(root.dataset.theme).toBe('sunset')
    applyPalette(DEFAULT_PALETTE)
    expect(root.hasAttribute('data-theme')).toBe(false)
  })

  it('reads the palette other windows saved', () => {
    expect(storedPalette()).toBe(DEFAULT_PALETTE)
    localStorage.setItem(PALETTE_STORAGE_KEY, 'forest')
    expect(storedPalette()).toBe('forest')
  })
})
