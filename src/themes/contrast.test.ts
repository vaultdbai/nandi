import { describe, expect, it } from 'vitest'
import css from '../theme-variables.css?raw'
import {
  contrastRatio,
  declarations,
  oklchToLinearRgb,
  over,
  resolveOklch,
  ruleBody,
  MASCOT_PAIRS,
  STATUS_PAIRS,
  TEXT_PAIRS,
  type Rgb,
} from './contrast'
import { PALETTES } from './palettes'

const light = declarations(ruleBody(css, ':root,\n.palette-preview'))
const dark = {
  ...light,
  ...declarations(ruleBody(css, '.dark,\n.dark .palette-preview')),
}

/** Overrides a palette states beyond its brand numbers */
function overrides(id: string, appearance: 'light' | 'dark') {
  const own = declarations(ruleBody(css, `html[data-theme='${id}']`))
  const darkOwn =
    appearance === 'dark'
      ? declarations(ruleBody(css, `html.dark[data-theme='${id}']`))
      : {}
  return { ...own, ...darkOwn }
}

/** A token's colour for one palette, following `var(--x)` references */
function colour(
  tokens: Record<string, string>,
  name: string,
  brand: { hue: number; chroma: number },
  depth = 0
): Rgb {
  const value = tokens[name]
  if (value === undefined || depth > 8) throw new Error(`No ${name}`)
  const ref = /^var\((--[\w-]+)\)$/.exec(value)
  if (ref) return colour(tokens, ref[1] as string, brand, depth + 1)
  const parsed = resolveOklch(value, brand)
  if (!parsed) throw new Error(`Can't read ${name}: ${value}`)
  const rgb = oklchToLinearRgb(parsed.l, parsed.c, parsed.h)
  if (parsed.alpha >= 1) return rgb
  // Translucent tokens sit on the page background
  return over(rgb, parsed.alpha, colour(tokens, '--background', brand))
}

const cases = PALETTES.flatMap(palette =>
  (['light', 'dark'] as const).map(appearance => ({
    palette,
    appearance,
    tokens: {
      ...(appearance === 'light' ? light : dark),
      ...overrides(palette.id, appearance),
    },
  }))
)

describe('theme contrast (WCAG AA)', () => {
  it.each(cases)(
    '$palette.id ($appearance): text is at least 4.5:1',
    ({ palette, tokens }) => {
      const brand = { hue: palette.hue, chroma: palette.chroma }
      const failing = TEXT_PAIRS.map(([fg, bg]) => ({
        pair: `${fg} on ${bg}`,
        ratio: contrastRatio(
          colour(tokens, fg, brand),
          colour(tokens, bg, brand)
        ),
      })).filter(result => result.ratio < 4.5)
      expect(failing).toEqual([])
    }
  )

  it.each(cases)(
    '$palette.id ($appearance): status colours are at least 3:1',
    ({ palette, tokens }) => {
      const brand = { hue: palette.hue, chroma: palette.chroma }
      const failing = STATUS_PAIRS.map(([fg, bg]) => ({
        pair: `${fg} on ${bg}`,
        ratio: contrastRatio(
          colour(tokens, fg, brand),
          colour(tokens, bg, brand)
        ),
      })).filter(result => result.ratio < 3)
      expect(failing).toEqual([])
    }
  )
})

describe('the mascot in every theme', () => {
  it.each(cases)(
    '$palette.id ($appearance): its outlines read at 3:1',
    ({ palette, tokens }) => {
      const brand = { hue: palette.hue, chroma: palette.chroma }
      const failing = MASCOT_PAIRS.map(([fg, bg]) => ({
        pair: `${fg} on ${bg}`,
        ratio: contrastRatio(
          colour(tokens, fg, brand),
          colour(tokens, bg, brand)
        ),
      })).filter(result => result.ratio < 3)
      expect(failing).toEqual([])
    }
  )
})

describe('contrast maths', () => {
  it('measures black on white as 21:1 and a colour on itself as 1:1', () => {
    const white = oklchToLinearRgb(1, 0, 0)
    const black = oklchToLinearRgb(0, 0, 0)
    expect(contrastRatio(white, black)).toBeCloseTo(21, 0)
    expect(contrastRatio(white, white)).toBe(1)
  })

  it('substitutes the brand numbers', () => {
    expect(
      resolveOklch('oklch(0.5 calc(var(--brand-c) * 0.5) var(--brand-h))', {
        hue: 200,
        chroma: 0.2,
      })
    ).toEqual({ l: 0.5, c: 0.1, h: 200, alpha: 1 })
    expect(
      resolveOklch('oklch(1 0 0 / 9%)', { hue: 0, chroma: 0 })?.alpha
    ).toBe(0.09)
  })
})
