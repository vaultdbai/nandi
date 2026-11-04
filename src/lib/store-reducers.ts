/**
 * Pure reducers that apply backend events to snapshot state.
 *
 * Every reducer is idempotent: messages are upserted by ID, and token
 * chunks carry the offset they were appended at, so a duplicate event is a
 * no-op. If an event can't be applied (e.g. a token arrives past the end of
 * a message we never saw), the reducer reports `resync` and the caller
 * reloads the snapshot — the DB is always the truth.
 */

import type {
  ActivityItem,
  AgentApprovalRequest,
  AgentApprovalResolved,
  AgentMessageAdded,
  AgentRunUpdated,
  AgentToken,
  AgentToolCall,
  AgentToolResult,
  AgentTurnDone,
  AttentionItem,
  ChatMessage,
  DashboardActivity,
  DashboardAttention,
  DashboardSnapshot,
  PendingApproval,
  RunSummary,
} from '@/lib/tauri-bindings'

/** A chat message, possibly sent but not yet confirmed by Rust */
export type UiMessage = ChatMessage & { pending?: boolean }

export interface ConversationState {
  conversationId: string
  messages: UiMessage[]
  currentRun: RunSummary | null
}

export type ConversationEvent =
  | { type: 'run'; payload: AgentRunUpdated }
  | { type: 'message'; payload: AgentMessageAdded }
  | { type: 'token'; payload: AgentToken }
  | { type: 'toolCall'; payload: AgentToolCall }
  | { type: 'toolResult'; payload: AgentToolResult }
  | { type: 'approvalRequest'; payload: AgentApprovalRequest }
  | { type: 'approvalResolved'; payload: AgentApprovalResolved }
  | { type: 'turnDone'; payload: AgentTurnDone }

export type DashboardEvent =
  | { type: 'attention'; payload: DashboardAttention }
  | { type: 'activity'; payload: DashboardActivity }

export type StoreEvent = ConversationEvent | DashboardEvent

/** Wire names, for the Activity log */
export const EVENT_NAMES: Record<StoreEvent['type'], string> = {
  run: 'agent://run',
  message: 'agent://message',
  token: 'agent://token',
  toolCall: 'agent://tool_call',
  toolResult: 'agent://tool_result',
  approvalRequest: 'agent://approval_request',
  approvalResolved: 'agent://approval_resolved',
  turnDone: 'agent://turn_done',
  attention: 'dashboard://attention',
  activity: 'dashboard://activity',
}

export const isConversationEvent = (
  event: StoreEvent
): event is ConversationEvent =>
  event.type !== 'attention' && event.type !== 'activity'

export interface ReduceResult {
  conversation: ConversationState
  /** The event couldn't be applied; reload the snapshot */
  resync: boolean
}

/** Inserts or replaces by ID, keeping position for replacements. */
export function upsertMessage(
  messages: UiMessage[],
  message: UiMessage
): UiMessage[] {
  const index = messages.findIndex(m => m.id === message.id)
  if (index === -1) return [...messages, message]
  return messages.map((m, i) => (i === index ? message : m))
}

const ok = (conversation: ConversationState): ReduceResult => ({
  conversation,
  resync: false,
})

export function reduceConversation(
  conversation: ConversationState,
  event: ConversationEvent
): ReduceResult {
  const withMessages = (messages: UiMessage[]) =>
    ok({ ...conversation, messages })

  switch (event.type) {
    case 'run': {
      const { run } = event.payload
      const isActive =
        run.status === 'running' || run.status === 'awaitingApproval'
      if (isActive) return ok({ ...conversation, currentRun: run })
      return conversation.currentRun?.id === run.id
        ? ok({ ...conversation, currentRun: null })
        : ok(conversation)
    }

    case 'message':
    case 'toolCall':
    case 'approvalRequest':
      return withMessages(
        upsertMessage(conversation.messages, event.payload.message)
      )

    case 'token': {
      const { messageId, offset, text, runId, createdAt } = event.payload
      const existing = conversation.messages.find(m => m.id === messageId)
      if (!existing) {
        if (offset !== 0) return { conversation, resync: true }
        return withMessages([
          ...conversation.messages,
          { kind: 'agent', id: messageId, runId, createdAt, content: text },
        ])
      }
      if (existing.kind !== 'agent') return ok(conversation)
      const length = existing.content.length
      // Already applied (duplicate delivery)
      if (offset + text.length <= length) return ok(conversation)
      // Missed a chunk
      if (offset !== length) return { conversation, resync: true }
      return withMessages(
        conversation.messages.map(m =>
          m.id === messageId && m.kind === 'agent'
            ? { ...m, content: m.content + text }
            : m
        )
      )
    }

    case 'toolResult': {
      const { messageId, result, isError } = event.payload
      if (!conversation.messages.some(m => m.id === messageId)) {
        return { conversation, resync: true }
      }
      return withMessages(
        conversation.messages.map(m =>
          m.id === messageId && m.kind === 'toolCall'
            ? { ...m, status: 'done', result, isError }
            : m
        )
      )
    }

    case 'approvalResolved': {
      const { approvalId, status } = event.payload
      return withMessages(
        conversation.messages.map(m =>
          m.id === approvalId && m.kind === 'approval' ? { ...m, status } : m
        )
      )
    }

    case 'turnDone': {
      const { runId, userMessageId } = event.payload
      return ok({
        ...conversation,
        currentRun:
          conversation.currentRun?.id === runId
            ? null
            : conversation.currentRun,
        messages: conversation.messages.map(m =>
          m.id === userMessageId && m.pending ? { ...m, pending: false } : m
        ),
      })
    }
  }
}

/** Keeps an agent's pending approvals in step with approval events. */
export function reducePendingApprovals(
  approvals: PendingApproval[],
  event: ConversationEvent
): PendingApproval[] {
  switch (event.type) {
    case 'approvalRequest': {
      const { approvalId, agentId, runId, conversationId, message } =
        event.payload
      if (message.kind !== 'approval') return approvals
      if (approvals.some(a => a.id === approvalId)) return approvals
      return [
        ...approvals,
        {
          id: approvalId,
          agentId,
          runId,
          conversationId,
          tool: message.tool,
          args: message.args,
          createdAt: message.createdAt,
          expiresAt: message.expiresAt,
        },
      ]
    }
    case 'approvalResolved':
      return approvals.filter(a => a.id !== event.payload.approvalId)
    default:
      return approvals
  }
}

/**
 * Keeps an agent's current turn fresh between `agent://queue_changed`
 * events (e.g. running → awaiting approval). One active run per agent.
 */
export function reduceCurrentTurn(
  current: RunSummary | null,
  event: ConversationEvent
): RunSummary | null {
  switch (event.type) {
    case 'run': {
      const { run } = event.payload
      if (run.status === 'running' || run.status === 'awaitingApproval') {
        return run
      }
      return current?.id === run.id ? null : current
    }
    case 'turnDone':
      return current?.id === event.payload.runId ? null : current
    default:
      return current
  }
}

/** Upserts a run into a newest-first list. */
export function upsertRun(runs: RunSummary[], run: RunSummary): RunSummary[] {
  const rest = runs.filter(r => r.id !== run.id)
  return [run, ...rest].sort((a, b) => b.startedAt - a.startedAt)
}

const RECENT_ACTIVITY_LIMIT = 20

export function reduceDashboard(
  dashboard: DashboardSnapshot,
  event: DashboardEvent
): DashboardSnapshot {
  switch (event.type) {
    case 'attention': {
      const delta = event.payload
      const items: AttentionItem[] =
        delta.op === 'upsert'
          ? [
              delta.item,
              ...dashboard.attentionItems.filter(i => i.id !== delta.item.id),
            ]
          : dashboard.attentionItems.filter(i => i.id !== delta.itemId)
      items.sort((a, b) => b.createdAt - a.createdAt)
      return {
        ...dashboard,
        attentionItems: items,
        stats: {
          ...dashboard.stats,
          needsAttention: items.filter(i => i.kind === 'approval').length,
        },
      }
    }
    case 'activity': {
      const { item } = event.payload
      const activity: ActivityItem[] = [
        item,
        ...dashboard.recentActivity.filter(a => a.id !== item.id),
      ]
        .sort((a, b) => b.createdAt - a.createdAt)
        .slice(0, RECENT_ACTIVITY_LIMIT)
      return { ...dashboard, recentActivity: activity }
    }
  }
}
