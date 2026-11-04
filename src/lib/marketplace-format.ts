import type { CostRange } from '@/lib/bindings'

/** `$0.01–$0.25` in the user's locale; `null` if there's no estimate */
export function formatCostRange(
  cost: CostRange | null,
  locale: string
): string | null {
  if (!cost) return null
  const format = (usd: number) =>
    new Intl.NumberFormat(locale, {
      style: 'currency',
      currency: 'USD',
      minimumFractionDigits: usd > 0 && usd < 0.1 ? 2 : 0,
      maximumFractionDigits: usd < 1 ? 2 : 0,
    }).format(usd)
  return cost.minUsd === cost.maxUsd
    ? format(cost.minUsd)
    : `${format(cost.minUsd)}–${format(cost.maxUsd)}`
}

/** `24 KB`, `1.2 MB` in the user's locale; `null` if unknown */
export function formatSize(kb: number | null, locale: string): string | null {
  if (kb === null) return null
  const unit = kb >= 1024 ? 'megabyte' : 'kilobyte'
  const value = kb >= 1024 ? kb / 1024 : kb
  return new Intl.NumberFormat(locale, {
    style: 'unit',
    unit,
    unitDisplay: 'short',
    maximumFractionDigits: 1,
  }).format(value)
}

/** Compares `x.y.z` versions (pre-release suffixes ignored). */
export function compareVersions(a: string, b: string): number {
  const parts = (version: string) =>
    version
      .replace(/^v/, '')
      .split(/[.+-]/)
      .slice(0, 3)
      .map(part => Number.parseInt(part, 10) || 0)
  const [x, y] = [parts(a), parts(b)]
  for (let i = 0; i < 3; i++) {
    const diff = (x[i] ?? 0) - (y[i] ?? 0)
    if (diff !== 0) return diff
  }
  return 0
}

/** Whether an entry matches the search (name, description, publisher, tags). */
export function matchesQuery(
  entry: {
    name: string
    description: string
    publisher: string
    tags: string[]
  },
  query: string
): boolean {
  const needle = query.trim().toLowerCase()
  if (!needle) return true
  return [entry.name, entry.description, entry.publisher, ...entry.tags].some(
    field => field.toLowerCase().includes(needle)
  )
}

/** `1.2.3` → `1.2.4`; anything else → `1.0.0` */
export function nextVersion(current: string | undefined): string {
  const match = current?.match(/^v?(\d+)\.(\d+)\.(\d+)/)
  if (!match) return '1.0.0'
  return `${match[1]}.${match[2]}.${Number(match[3]) + 1}`
}
