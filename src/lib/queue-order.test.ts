import { describe, expect, it } from 'vitest'
import { compareQueueEntries, priorityForMove } from './queue-order'
import { queueEntry, T0 } from '@/test/fixtures'
import type { QueueEntry } from '@/lib/tauri-bindings'

const entry = (runId: string, priority: number, age: number) =>
  queueEntry(runId, 'writer', { priority, createdAt: T0 + age })

/** Applies a move the way Rust would order it afterwards */
function move(queue: QueueEntry[], from: number, to: number): string[] {
  const priority = priorityForMove(queue, from, to)
  return queue
    .map((e, i) => (i === from && priority !== null ? { ...e, priority } : e))
    .sort(compareQueueEntries)
    .map(e => e.runId)
}

describe('compareQueueEntries', () => {
  it('orders by priority, then oldest first', () => {
    const queue = [
      entry('bg-old', 50, 1),
      entry('user-new', 10, 4),
      entry('bg-new', 50, 3),
      entry('user-old', 10, 2),
    ]
    expect([...queue].sort(compareQueueEntries).map(e => e.runId)).toEqual([
      'user-old',
      'user-new',
      'bg-old',
      'bg-new',
    ])
  })
})

describe('priorityForMove', () => {
  const queue = [entry('a', 10, 1), entry('b', 50, 2), entry('c', 80, 3)]

  it('ignores no-op and out-of-range moves', () => {
    expect(priorityForMove(queue, 1, 1)).toBeNull()
    expect(priorityForMove(queue, 0, 5)).toBeNull()
    expect(priorityForMove([], 0, 0)).toBeNull()
  })

  it('moves to the top, middle, and bottom', () => {
    expect(move(queue, 2, 0)).toEqual(['c', 'a', 'b'])
    expect(move(queue, 0, 1)).toEqual(['b', 'a', 'c'])
    expect(move(queue, 0, 2)).toEqual(['b', 'c', 'a'])
    expect(move(queue, 2, 1)).toEqual(['a', 'c', 'b'])
  })

  it("prefers a neighbour's priority when the age tie-break suffices", () => {
    // `c` is newer than `a`, so matching a's priority puts it right after a
    expect(priorityForMove(queue, 2, 1)).toBe(10)
    // `a` is older than `c`, so matching c's priority puts it right before c
    expect(priorityForMove(queue, 0, 1)).toBe(80)
  })

  it('works between neighbours with equal priorities', () => {
    const same = [entry('a', 10, 1), entry('b', 10, 2), entry('c', 10, 3)]
    expect(move(same, 2, 0)).toEqual(['c', 'a', 'b'])
    expect(move(same, 0, 2)).toEqual(['b', 'c', 'a'])
    // An equal-priority group can't be split by age: it goes past `a`
    // rather than staying put
    expect(move(same, 2, 1)).toEqual(['c', 'a', 'b'])
  })
})
