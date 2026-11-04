/** The delegation thread's model: one time-ordered list from a task, its events, and its messages */
import type {
  JsonValue,
  TaskEvent,
  TaskMessage,
  TaskRecord,
} from '@/lib/bindings'
import { isTerminalState } from '@/hooks/useTasks'

/** One entry of the thread, in time order */
export type ThreadItem =
  | { kind: 'delegate'; at: number; task: TaskRecord }
  | { kind: 'state'; at: number; event: TaskEvent }
  | { kind: 'message'; at: number; message: TaskMessage }
  | { kind: 'result'; at: number; task: TaskRecord }

/**
 * The task, its state transitions, and its messages as one chronological
 * thread: the delegation first, the result (once terminal) last.
 */
export function buildThread(
  task: TaskRecord,
  events: TaskEvent[],
  messages: TaskMessage[]
): ThreadItem[] {
  const middle: ThreadItem[] = [
    ...events.map(event => ({ kind: 'state' as const, at: event.at, event })),
    ...messages.map(message => ({
      kind: 'message' as const,
      at: message.at,
      message,
    })),
  ].sort((a, b) => a.at - b.at)
  const items: ThreadItem[] = [
    { kind: 'delegate', at: task.createdAt, task },
    ...middle,
  ]
  if (isTerminalState(task.state)) {
    items.push({
      kind: 'result',
      at: task.completedAt ?? task.updatedAt,
      task,
    })
  }
  return items
}

interface Usage {
  tokens: number | null
  costUsd: number | null
}

export const asRecord = (value: JsonValue | null | undefined) =>
  value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, JsonValue>)
    : null

export const asNumber = (value: JsonValue | undefined): number | null =>
  typeof value === 'number' && Number.isFinite(value) ? value : null

/** Tokens and cost, when the provider's result reports them */
export function extractUsage(result: JsonValue | null): Usage {
  const record = asRecord(result)
  if (!record) return { tokens: null, costUsd: null }
  const usage = asRecord(record.usage) ?? record
  const input = asNumber(usage.inputTokens)
  const output = asNumber(usage.outputTokens)
  const tokens =
    asNumber(usage.totalTokens) ??
    asNumber(usage.tokens) ??
    (input !== null || output !== null ? (input ?? 0) + (output ?? 0) : null)
  const costUsd =
    asNumber(usage.costUsd) ?? asNumber(record.costUsd) ?? asNumber(usage.cost)
  return { tokens, costUsd }
}
