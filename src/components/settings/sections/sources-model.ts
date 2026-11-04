/** The two kinds of source Settings › Sources lists */
export type SourceKind = 'agents' | 'capabilities'

const DAY_MS = 24 * 60 * 60 * 1000

/** Not synced for over a day (shown amber); never-synced isn't "stale" */
export function isStale(syncedAt: number | null, now = Date.now()): boolean {
  return syncedAt !== null && now - syncedAt > DAY_MS
}
