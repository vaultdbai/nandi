/**
 * An ask to a person made from a chat (`@Sam …`), as its card says it:
 * sent, seen, answered or not, and the answer's words. Pure; the state and
 * the result are its delegated task's.
 */
import type { JsonValue, TaskState } from '@/lib/bindings'

export type AskStage = 'sent' | 'seen' | 'answered' | 'notAnswered'

export function askStage(state: TaskState): AskStage {
  switch (state) {
    case 'submitted':
    case 'delivered':
    case 'reassigned':
      return 'sent'
    case 'acknowledged':
    case 'working':
    case 'inputRequired':
      return 'seen'
    case 'completed':
      return 'answered'
    default:
      return 'notAnswered'
  }
}

/** The answer in words: its text, or the result as it came */
export function answerText(result: JsonValue | null): string | null {
  if (result === null || result === undefined) return null
  if (typeof result === 'string') return result.trim() || null
  if (typeof result === 'object' && !Array.isArray(result)) {
    for (const key of ['text', 'answer', 'output', 'summary']) {
      const value = (result as Record<string, JsonValue>)[key]
      if (value !== undefined && value !== null) {
        const text = answerText(value)
        if (text) return text
      }
    }
  }
  return JSON.stringify(result)
}
