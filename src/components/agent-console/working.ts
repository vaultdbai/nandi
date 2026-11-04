import type { RunSummary } from '@/lib/tauri-bindings'
import type { UiMessage } from '@/types/agent'

/**
 * Whether the assistant is on a turn: its run is queued, running or held
 * for an approval, or the user's message is sent but not confirmed yet
 */
export function isWorking(
  run: Pick<RunSummary, 'status'> | null | undefined,
  messages: UiMessage[]
): boolean {
  if (messages.at(-1)?.pending) return true
  return (
    run?.status === 'queued' ||
    run?.status === 'running' ||
    run?.status === 'awaitingApproval'
  )
}

/**
 * Whether the working dots belong at the end: not while the reply's text
 * is streaming (that shows progress itself), and not while an approval
 * waits for the user (it's their move, not the agent's).
 */
export function showsWorking(messages: UiMessage[], working: boolean) {
  if (!working) return false
  const last = messages.at(-1)
  if (last?.kind === 'agent' && last.content.length > 0) return false
  if (last?.kind === 'approval' && last.status === 'pending') return false
  return true
}

/** When the current turn began: the user's last message (epoch ms) */
export function turnStartedAt(messages: UiMessage[]): number | undefined {
  for (let i = messages.length - 1; i >= 0; i--) {
    const message = messages[i]
    if (message?.kind === 'user') return message.createdAt
  }
  return undefined
}
