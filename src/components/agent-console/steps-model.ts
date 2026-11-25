/**
 * Steps quiet, decisions loud: a live chat folds each stretch of tool
 * calls under "Show steps", and keeps the agent's words, approval cards,
 * errors and the plan card in full. Pure; `MessageList` renders it.
 */
import type { UiMessage } from '@/types/agent'
import { isDelegation } from './delegation-model'
import { isPlanCard } from './run-plan-model'

export type ChatItem =
  | { kind: 'message'; message: UiMessage }
  | { kind: 'steps'; id: string; messages: UiMessage[] }

/**
 * A step: a tool call that isn't the plan card, nor another agent's work
 * (that reads like a person in the chat)
 */
const isStep = (message: UiMessage): boolean =>
  message.kind === 'toolCall' && !isPlanCard(message) && !isDelegation(message)

/** Messages as shown, with each run of steps folded into one item */
export function foldSteps(messages: readonly UiMessage[]): ChatItem[] {
  const items: ChatItem[] = []
  for (const message of messages) {
    const last = items.at(-1)
    if (!isStep(message)) {
      items.push({ kind: 'message', message })
    } else if (last?.kind === 'steps') {
      last.messages.push(message)
    } else {
      items.push({
        kind: 'steps',
        id: `steps:${message.id}`,
        messages: [message],
      })
    }
  }
  return items
}
