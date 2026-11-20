/**
 * The GitHub security check's line in Settings › Account: when it last
 * ran and what it found (the findings themselves are `check` to-dos).
 * Pure, for tests.
 */
import type { GithubSecurityStatus } from '@/lib/bindings'
import { formatRelativeTime } from '@/lib/format-time'

export interface SecurityLine {
  /** `github.security.<…>` */
  key: string
  values: { when?: string; count?: number }
}

/** "Never checked", "Checked 2 hr. ago: nothing to fix" or "…: 3 to-dos" */
export function securityLine(
  status: GithubSecurityStatus | undefined,
  locale: string,
  now: Date = new Date()
): SecurityLine {
  if (status?.checkedAt == null)
    return { key: 'github.security.never', values: {} }
  const when = formatRelativeTime(status.checkedAt, locale, now)
  if (status.findings === 0)
    return { key: 'github.security.clean', values: { when } }
  return {
    key: 'github.security.found',
    values: { when, count: status.findings },
  }
}
