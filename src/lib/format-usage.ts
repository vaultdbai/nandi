/** "12.4K", localized */
export function formatTokenCount(count: number, locale: string): string {
  return new Intl.NumberFormat(locale, {
    notation: 'compact',
    maximumFractionDigits: 1,
  }).format(count)
}

/** "$0.0042" for fractions of a cent, "$1.23" otherwise */
export function formatCostUsd(cost: number, locale: string): string {
  return new Intl.NumberFormat(locale, {
    style: 'currency',
    currency: 'USD',
    ...(cost > 0 && cost < 0.01
      ? { maximumSignificantDigits: 2 }
      : { minimumFractionDigits: 2, maximumFractionDigits: 2 }),
  }).format(cost)
}
