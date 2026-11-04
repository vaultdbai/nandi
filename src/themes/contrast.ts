/**
 * WCAG contrast for the theme tokens. The tokens are oklch() colours,
 * some derived from a palette's brand hue and chroma
 * (`calc(var(--brand-c) * k)`), so this resolves them for one palette
 * and appearance, converts to sRGB, and measures the ratio. Pure, so
 * tests check every built-in palette in light and dark.
 */

export interface Rgb {
  r: number
  g: number
  b: number
}

/** A token's value with the brand numbers substituted, then evaluated */
export function resolveOklch(
  value: string,
  brand: { hue: number; chroma: number }
): { l: number; c: number; h: number; alpha: number } | null {
  const substituted = value
    .replace(/var\(--brand-h\)/g, String(brand.hue))
    .replace(/var\(--brand-c\)/g, String(brand.chroma))
    .replace(/calc\(([^()]+)\)/g, (_, expression: string) => {
      const [a, op, b] = expression.trim().split(/\s+/)
      const x = Number(a)
      const y = Number(b)
      if (op === '*') return String(x * y)
      if (op === '+') return String(x + y)
      if (op === '-') return String(x - y)
      if (op === '/') return String(x / y)
      return 'NaN'
    })
  const match =
    /oklch\(\s*([\d.]+)%?\s+([\d.]+)\s+([\d.]+)\s*(?:\/\s*([\d.]+)(%?))?\s*\)/.exec(
      substituted
    )
  if (!match) return null
  const alpha =
    match[4] === undefined ? 1 : Number(match[4]) / (match[5] === '%' ? 100 : 1)
  return {
    l: Number(match[1]),
    c: Number(match[2]),
    h: Number(match[3]),
    alpha,
  }
}

/** oklch → linear sRGB (clipped to the gamut) */
export function oklchToLinearRgb(l: number, c: number, h: number): Rgb {
  const rad = (h * Math.PI) / 180
  const a = c * Math.cos(rad)
  const b = c * Math.sin(rad)
  const l_ = l + 0.3963377774 * a + 0.2158037573 * b
  const m_ = l - 0.1055613458 * a - 0.0638541728 * b
  const s_ = l - 0.0894841775 * a - 1.291485548 * b
  const L = l_ ** 3
  const M = m_ ** 3
  const S = s_ ** 3
  const clip = (v: number) => Math.min(1, Math.max(0, v))
  return {
    r: clip(4.0767416621 * L - 3.3077115913 * M + 0.2309699292 * S),
    g: clip(-1.2684380046 * L + 2.6097574011 * M - 0.3413193965 * S),
    b: clip(-0.0041960863 * L - 0.7034186147 * M + 1.707614701 * S),
  }
}

/** WCAG relative luminance of a linear sRGB colour */
export function luminance({ r, g, b }: Rgb): number {
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

/** WCAG contrast ratio (1–21) */
export function contrastRatio(a: Rgb, b: Rgb): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x) as [
    number,
    number,
  ]
  return (hi + 0.05) / (lo + 0.05)
}

/** A colour over an opaque background (alpha blended in linear light) */
export function over(top: Rgb, alpha: number, bottom: Rgb): Rgb {
  return {
    r: top.r * alpha + bottom.r * (1 - alpha),
    g: top.g * alpha + bottom.g * (1 - alpha),
    b: top.b * alpha + bottom.b * (1 - alpha),
  }
}

/** The custom-property declarations in one CSS block */
export function declarations(block: string): Record<string, string> {
  const out: Record<string, string> = {}
  // Values may span lines (prettier wraps long oklch() calls)
  for (const match of block.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) {
    out[match[1] as string] = (match[2] as string).replace(/\s+/g, ' ').trim()
  }
  return out
}

/** The body of the first rule whose selector list is exactly `selector` */
export function ruleBody(css: string, selector: string): string {
  const pattern = new RegExp(
    `(^|\\n)${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`
  )
  return pattern.exec(css)?.[2] ?? ''
}

/** Text and icon pairs every theme must keep readable */
export const TEXT_PAIRS: [string, string][] = [
  ['--foreground', '--background'],
  ['--card-foreground', '--card'],
  ['--popover-foreground', '--popover'],
  ['--muted-foreground', '--background'],
  ['--muted-foreground', '--card'],
  ['--primary-foreground', '--primary'],
  ['--secondary-foreground', '--secondary'],
  ['--accent-foreground', '--accent'],
  ['--sidebar-foreground', '--sidebar'],
  ['--sidebar-muted-foreground', '--sidebar'],
  ['--sidebar-accent-foreground', '--sidebar-accent'],
]

/** The mascot's outlines on its body (a graphic: 3:1) */
export const MASCOT_PAIRS: [string, string][] = [
  ['--nandi-line', '--nandi-body'],
  ['--nandi-line', '--background'],
]

/** Status colours used as text and icons on the page */
export const STATUS_PAIRS: [string, string][] = [
  ['--success', '--background'],
  ['--warning', '--background'],
  ['--info', '--background'],
  ['--destructive', '--background'],
  ['--success', '--card'],
  ['--warning', '--card'],
  ['--info', '--card'],
  ['--destructive', '--card'],
]
