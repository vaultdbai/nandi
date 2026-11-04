import { create } from 'zustand'
import { devtools } from 'zustand/middleware'
import type {
  AgentInfo,
  AgentSnapshot,
  ContainerState,
  ConversationSnapshot,
  DashboardSnapshot,
  PendingApproval,
  QueueEntry,
  RunEventRecord,
  RunSummary,
  StudioConversationSummary,
} from '@/lib/tauri-bindings'
import {
  EVENT_NAMES,
  reduceConversation,
  reduceCurrentTurn,
  reduceDashboard,
  reducePendingApprovals,
  upsertMessage,
  upsertRun,
  type ConversationEvent,
  type ConversationState,
  type DashboardEvent,
  type UiMessage,
} from '@/lib/store-reducers'

/** Per-agent cap on the Activity log */
export const MAX_AGENT_EVENTS = 200

/** Live events get negative IDs so they never collide with DB row IDs */
let liveEventSeq = 0

/** Everything about an agent except its messages (see `conversations`) */
export interface AgentState {
  agent: AgentInfo | null
  conversationId: string
  pendingApprovals: PendingApproval[]
  recentRuns: RunSummary[]
  recentEvents: RunEventRecord[]
  container: ContainerState
  /**
   * Latest `agent://warning` (e.g. tools unavailable). Not in snapshots:
   * cleared when dismissed or once the container is running.
   */
  warning: string | null
}

/**
 * Mirror of Rust state: snapshots plus event deltas, nothing else. Every
 * field here can be rebuilt from `get_*` snapshot commands. Loading and
 * event subscription live in `src/lib/agent-sync.ts`.
 */
interface AgentStoreState {
  agents: Record<string, AgentState>
  /** Agent and Studio conversations, by conversation ID */
  conversations: Record<string, ConversationState>
  dashboard: DashboardSnapshot | null
  studio: StudioConversationSummary[] | null
  /** Each agent's waiting runs, next to run first (by agent ID) */
  queue: Record<string, QueueEntry[]>
  /** Each agent's running or approval-blocked turn (by agent ID) */
  current: Record<string, RunSummary | null>

  setAgentSnapshot: (snapshot: AgentSnapshot, agentId: string) => void
  setAgentWarning: (agentId: string, warning: string | null) => void
  /** From `agent://queue_changed`, which carries the whole state */
  setQueue: (
    agentId: string,
    queue: QueueEntry[],
    current: RunSummary | null
  ) => void
  setConversation: (snapshot: ConversationSnapshot) => void
  setDashboard: (dashboard: DashboardSnapshot) => void
  setStudio: (conversations: StudioConversationSummary[]) => void
  upsertStudioConversation: (conversation: StudioConversationSummary) => void
  /** Applies an event; returns false if the snapshot needs a reload */
  applyConversationEvent: (event: ConversationEvent) => boolean
  applyDashboardEvent: (event: DashboardEvent) => void
  /** Optimistic user message, before `send_message` confirms it */
  addPendingMessage: (conversationId: string, message: UiMessage) => void
  confirmMessage: (conversationId: string, messageId: string) => void
  removeMessage: (conversationId: string, messageId: string) => void
  /** Removes an optimistic message unless Rust has confirmed it */
  dropPendingMessage: (conversationId: string, messageId: string) => void
}

const fromSnapshot = (snapshot: ConversationSnapshot): ConversationState => ({
  conversationId: snapshot.conversationId,
  messages: snapshot.messages,
  currentRun: snapshot.currentRun,
})

/** Keeps optimistic messages the snapshot doesn't know about yet */
const mergePending = (
  previous: ConversationState | undefined,
  next: ConversationState
): ConversationState => {
  const pending = (previous?.messages ?? []).filter(
    m => m.pending && !next.messages.some(n => n.id === m.id)
  )
  return pending.length > 0
    ? { ...next, messages: [...next.messages, ...pending] }
    : next
}

export const useAgentStore = create<AgentStoreState>()(
  devtools(
    (set, get) => ({
      agents: {},
      conversations: {},
      dashboard: null,
      studio: null,
      queue: {},
      current: {},

      setAgentSnapshot: (snapshot, agentId) =>
        set(
          state => ({
            agents: {
              ...state.agents,
              [agentId]: {
                agent: snapshot.agent,
                conversationId: snapshot.conversation.conversationId,
                pendingApprovals: snapshot.pendingApprovals,
                recentRuns: snapshot.recentRuns,
                recentEvents: snapshot.recentEvents,
                container: snapshot.container,
                warning:
                  snapshot.container.status === 'running'
                    ? null
                    : (state.agents[agentId]?.warning ?? null),
              },
            },
            conversations: {
              ...state.conversations,
              [snapshot.conversation.conversationId]: mergePending(
                state.conversations[snapshot.conversation.conversationId],
                fromSnapshot(snapshot.conversation)
              ),
            },
            queue: { ...state.queue, [agentId]: snapshot.queue },
            current: { ...state.current, [agentId]: snapshot.currentTurn },
          }),
          undefined,
          'setAgentSnapshot'
        ),

      setQueue: (agentId, queue, current) =>
        set(
          state => ({
            queue: { ...state.queue, [agentId]: queue },
            current: { ...state.current, [agentId]: current },
          }),
          undefined,
          'setQueue'
        ),

      setAgentWarning: (agentId, warning) =>
        set(
          state => {
            const agent = state.agents[agentId]
            if (!agent) return state
            return {
              agents: { ...state.agents, [agentId]: { ...agent, warning } },
            }
          },
          undefined,
          'setAgentWarning'
        ),

      setConversation: snapshot =>
        set(
          state => ({
            conversations: {
              ...state.conversations,
              [snapshot.conversationId]: mergePending(
                state.conversations[snapshot.conversationId],
                fromSnapshot(snapshot)
              ),
            },
          }),
          undefined,
          'setConversation'
        ),

      setDashboard: dashboard => set({ dashboard }, undefined, 'setDashboard'),

      setStudio: studio => set({ studio }, undefined, 'setStudio'),

      upsertStudioConversation: conversation =>
        set(
          state => ({
            studio: [
              conversation,
              ...(state.studio ?? []).filter(c => c.id !== conversation.id),
            ],
          }),
          undefined,
          'upsertStudioConversation'
        ),

      applyConversationEvent: event => {
        const { agentId, conversationId } = event.payload
        const conversation = get().conversations[conversationId]
        const agent = get().agents[agentId]
        let resync = false

        set(
          state => {
            const next: Partial<AgentStoreState> = {}
            const current = state.current[agentId] ?? null
            const nextCurrent = reduceCurrentTurn(current, event)
            if (nextCurrent !== current) {
              next.current = { ...state.current, [agentId]: nextCurrent }
            }
            if (conversation) {
              const result = reduceConversation(conversation, event)
              resync = result.resync
              next.conversations = {
                ...state.conversations,
                [conversationId]: result.conversation,
              }
            }
            if (agent) {
              next.agents = {
                ...state.agents,
                [agentId]: {
                  ...agent,
                  pendingApprovals: reducePendingApprovals(
                    agent.pendingApprovals,
                    event
                  ),
                  recentRuns:
                    event.type === 'run'
                      ? upsertRun(agent.recentRuns, event.payload.run)
                      : agent.recentRuns,
                  // Tokens aren't recorded in run_events either
                  recentEvents:
                    event.type === 'token'
                      ? agent.recentEvents
                      : [
                          ...agent.recentEvents,
                          {
                            id: -++liveEventSeq,
                            runId: eventRunId(event),
                            kind: EVENT_NAMES[event.type],
                            payload: event.payload,
                            createdAt: Date.now(),
                          } satisfies RunEventRecord,
                        ].slice(-MAX_AGENT_EVENTS),
                },
              }
            }
            return next
          },
          undefined,
          `applyConversationEvent/${event.type}`
        )
        return !resync
      },

      applyDashboardEvent: event =>
        set(
          state =>
            state.dashboard
              ? { dashboard: reduceDashboard(state.dashboard, event) }
              : state,
          undefined,
          `applyDashboardEvent/${event.type}`
        ),

      addPendingMessage: (conversationId, message) =>
        set(
          state => {
            const conversation = state.conversations[conversationId] ?? {
              conversationId,
              messages: [],
              currentRun: null,
            }
            return {
              conversations: {
                ...state.conversations,
                [conversationId]: {
                  ...conversation,
                  messages: upsertMessage(conversation.messages, {
                    ...message,
                    pending: true,
                  }),
                },
              },
            }
          },
          undefined,
          'addPendingMessage'
        ),

      confirmMessage: (conversationId, messageId) =>
        set(
          state => {
            const conversation = state.conversations[conversationId]
            if (!conversation) return state
            return {
              conversations: {
                ...state.conversations,
                [conversationId]: {
                  ...conversation,
                  messages: conversation.messages.map(m =>
                    m.id === messageId ? { ...m, pending: false } : m
                  ),
                },
              },
            }
          },
          undefined,
          'confirmMessage'
        ),

      removeMessage: (conversationId, messageId) =>
        set(
          state => {
            const conversation = state.conversations[conversationId]
            if (!conversation) return state
            return {
              conversations: {
                ...state.conversations,
                [conversationId]: {
                  ...conversation,
                  messages: conversation.messages.filter(
                    m => m.id !== messageId
                  ),
                },
              },
            }
          },
          undefined,
          'removeMessage'
        ),

      dropPendingMessage: (conversationId, messageId) =>
        set(
          state => {
            const conversation = state.conversations[conversationId]
            if (!conversation) return state
            return {
              conversations: {
                ...state.conversations,
                [conversationId]: {
                  ...conversation,
                  messages: conversation.messages.filter(
                    m => !(m.id === messageId && m.pending)
                  ),
                },
              },
            }
          },
          undefined,
          'dropPendingMessage'
        ),
    }),
    { name: 'agent-store' }
  )
)

function eventRunId(event: ConversationEvent): string {
  switch (event.type) {
    case 'run':
      return event.payload.run.id
    case 'message':
      return event.payload.message.runId ?? ''
    default:
      return event.payload.runId
  }
}

// ---------------------------------------------------------------------------
// Selectors
// ---------------------------------------------------------------------------

// Stable fallbacks so selectors don't return a new array each render
const NO_MESSAGES: UiMessage[] = []
const NO_APPROVALS: PendingApproval[] = []
const NO_QUEUE: QueueEntry[] = []

/** The agent's Rust-side state, or undefined until its snapshot loads */
export const useAgent = (agentId: string) =>
  useAgentStore(state => state.agents[agentId])

export const useMessages = (agentId: string) =>
  useAgentStore(state => {
    const conversationId = state.agents[agentId]?.conversationId
    return (
      (conversationId && state.conversations[conversationId]?.messages) ||
      NO_MESSAGES
    )
  })

export const useCurrentRun = (agentId: string) =>
  useAgentStore(state => {
    const conversationId = state.agents[agentId]?.conversationId
    return (
      (conversationId && state.conversations[conversationId]?.currentRun) ||
      null
    )
  })

/** The agent's queued runs, next to run first */
export const useAgentQueue = (agentId: string) =>
  useAgentStore(state => state.queue[agentId] ?? NO_QUEUE)

/** The agent's running or approval-blocked turn, or null when idle */
export const useCurrentTurn = (agentId: string) =>
  useAgentStore(state => state.current[agentId] ?? null)

export const usePendingApprovals = (agentId: string) =>
  useAgentStore(
    state => state.agents[agentId]?.pendingApprovals ?? NO_APPROVALS
  )

export const useConversation = (conversationId: string | null) =>
  useAgentStore(state =>
    conversationId ? state.conversations[conversationId] : undefined
  )

export const useDashboard = () => useAgentStore(state => state.dashboard)

export const useStudioConversations = () => useAgentStore(state => state.studio)
