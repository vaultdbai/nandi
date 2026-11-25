/**
 * Delegated work inside the parent chat: which of another agent's runs a
 * `delegate_to_agent` call started. An agent runs one turn at a time and
 * its tool calls one after another, so the runs it delegated while a call
 * went on are that call's. Pure; `DelegationBlock` renders it.
 */
import type { RunSummary } from '@/lib/bindings'
import type { UiMessage } from '@/types/agent'

/** The built-in delegation tool (`a2a::tool::NAME`) */
export const DELEGATE_TOOL = 'delegate_to_agent'

export function isDelegation(message: UiMessage): boolean {
  return (
    message.kind === 'toolCall' &&
    message.tool === DELEGATE_TOOL &&
    message.runId !== null
  )
}

/**
 * For each delegation call, when the next one in its run started (its
 * window ends there), or `null` for the run's last
 */
export function delegationWindows(
  messages: readonly UiMessage[]
): Map<string, number | null> {
  const windows = new Map<string, number | null>()
  const last = new Map<string, string>()
  for (const message of messages) {
    if (!isDelegation(message) || message.runId === null) continue
    const previous = last.get(message.runId)
    if (previous) windows.set(previous, message.createdAt)
    windows.set(message.id, null)
    last.set(message.runId, message.id)
  }
  return windows
}

/** The delegated runs a call started: in its window, oldest first */
export function runsOfCall(
  runs: readonly RunSummary[],
  from: number,
  until: number | null
): RunSummary[] {
  return runs
    .filter(
      run => run.startedAt >= from && (until === null || run.startedAt < until)
    )
    .sort((a, b) => a.startedAt - b.startedAt)
}

/** "<Agent> is working on it", "… needs you", "… is done", "… stopped" */
export function delegationState(
  status: RunSummary['status']
): 'working' | 'needsYou' | 'done' | 'stopped' {
  switch (status) {
    case 'awaitingApproval':
      return 'needsYou'
    case 'completed':
      return 'done'
    case 'failed':
    case 'cancelled':
      return 'stopped'
    default:
      return 'working'
  }
}

/** The child's approvals still waiting: shown open, so the person decides there */
export function splitChildMessages(messages: readonly UiMessage[]): {
  waiting: UiMessage[]
  steps: UiMessage[]
} {
  const waiting = messages.filter(
    m => m.kind === 'approval' && m.status === 'pending'
  )
  return {
    waiting,
    steps: messages.filter(m => !waiting.includes(m)),
  }
}
