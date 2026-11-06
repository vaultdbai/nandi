import type { PullProgress } from '@/lib/bindings'
import { formatSize } from '@/lib/marketplace-format'

/** `456 MB` in the user's locale; `null` when the size isn't known */
export function formatDownload(
  bytes: number | null | undefined,
  locale: string
): string | null {
  if (bytes === null || bytes === undefined || bytes <= 0) return null
  return formatSize(bytes / 1024, locale)
}

/** 0–100, or `null` while the total isn't known */
export function progressPercent(
  progress: PullProgress | null | undefined
): number | null {
  if (!progress?.totalBytes) return null
  const percent = (progress.doneBytes / progress.totalBytes) * 100
  return Math.min(100, Math.max(0, Math.round(percent)))
}
