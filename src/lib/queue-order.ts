import type { QueueEntry } from '@/lib/tauri-bindings'

/** The Rust queue's order: lower priority first, then oldest. */
export function compareQueueEntries(a: QueueEntry, b: QueueEntry): number {
  return a.priority - b.priority || a.createdAt - b.createdAt
}

/**
 * The priority that moves `queue[from]` to index `to` (a drag in the queue
 * panel), or `null` if nothing needs to change. Prefers a neighbour's
 * priority, so priorities stay meaningful, then a value between the two.
 */
export function priorityForMove(
  queue: QueueEntry[],
  from: number,
  to: number
): number | null {
  const moved = queue[from]
  if (!moved || from === to || to < 0 || to >= queue.length) return null
  const rest = queue.filter((_, i) => i !== from)
  const above = rest[to - 1]
  const below = rest[to]

  const candidates: number[] = []
  if (above) candidates.push(above.priority)
  if (below) candidates.push(below.priority)
  if (above && below && below.priority - above.priority >= 2) {
    candidates.push(Math.floor((above.priority + below.priority) / 2))
  }
  if (above) candidates.push(above.priority + 1)
  if (below) candidates.push(below.priority - 1)

  const landsAt = (priority: number) => {
    const entry = { ...moved, priority }
    const sorted = [...rest, entry].sort(compareQueueEntries)
    return sorted.indexOf(entry)
  }
  const exact = candidates.find(priority => landsAt(priority) === to)
  if (exact !== undefined) return exact
  // No integer fits between equal neighbours created in this order: get
  // as close as possible
  return below ? below.priority - 1 : (above?.priority ?? 0) + 1
}
