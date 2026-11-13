/**
 * The companion's own data: the snapshot (orb blocker, pin, "Notify me",
 * the conversation and its scope) and its chat, a Studio conversation that
 * answers from Studio's tools. Its lists come from Home's queries
 * (`useInbox`, `useHomeFeed`) instead.
 *
 * The companion is its own webview, so this hook subscribes to the core's
 * events itself (call it once per window) and keeps the conversation in
 * that webview's agent store.
 */

import { useEffect, useState } from 'react'
import {
  useQuery,
  useQueryClient,
  type QueryClient,
} from '@tanstack/react-query'
import { client, type NandiEvent } from '@/lib/client'
import { logger } from '@/lib/logger'
import type { ConversationEvent, UiMessage } from '@/lib/store-reducers'
import type { CompanionSnapshot } from '@/lib/bindings'
import { useAgentStore, useConversation } from '@/store/agent-store'

const STUDIO_AGENT_ID = '@studio'

export const companionQueryKeys = {
  all: ['companion'] as const,
  snapshot: () => [...companionQueryKeys.all, 'snapshot'] as const,
}

/** Events that change the snapshot's counts */
const SNAPSHOT_EVENTS = new Set<NandiEvent['event']>([
  'agent://run',
  'agent://turn_done',
  'agent://approval_request',
  'agent://approval_resolved',
  'agent://agents_changed',
  'dashboard://attention',
  'dashboard://activity',
  // A workflow waits on (or resumed from) a manual step
  'workflow://manual_step_ready',
  'workflow://manual_step_completed',
  'nandi://resync',
])
const REFRESH_DELAY_MS = 250

/** Start of today in local time (Rust doesn't know the user's timezone) */
export function localDayStart(now = new Date()): number {
  const date = new Date(now)
  date.setHours(0, 0, 0, 0)
  return date.getTime()
}

function toConversationEvent(event: NandiEvent): ConversationEvent | null {
  switch (event.event) {
    case 'agent://run':
      return { type: 'run', payload: event.payload }
    case 'agent://message':
      return { type: 'message', payload: event.payload }
    case 'agent://token':
      return { type: 'token', payload: event.payload }
    case 'agent://tool_call':
      return { type: 'toolCall', payload: event.payload }
    case 'agent://tool_result':
      return { type: 'toolResult', payload: event.payload }
    case 'agent://approval_request':
      return { type: 'approvalRequest', payload: event.payload }
    case 'agent://approval_resolved':
      return { type: 'approvalResolved', payload: event.payload }
    case 'agent://turn_done':
      return { type: 'turnDone', payload: event.payload }
    default:
      return null
  }
}

async function loadStudioConversation(conversationId: string) {
  const result = await client.getConversation(conversationId)
  if (result.status === 'error') {
    logger.warn('Companion: failed to load conversation', {
      conversationId,
      error: result.error,
    })
    return
  }
  useAgentStore.getState().setConversation(result.data)
}

/**
 * Subscribes to core events for the companion webview: refreshes the
 * snapshot (debounced) and applies conversation deltas to the
 * conversations it has loaded. Returns the unsubscribe function.
 */
export function subscribeCompanionEvents(queryClient: QueryClient) {
  let timer: ReturnType<typeof setTimeout> | undefined
  const refresh = () => {
    clearTimeout(timer)
    // The snapshot and the feed (`useCompanionFeed`)
    timer = setTimeout(() => {
      void queryClient.invalidateQueries({
        queryKey: companionQueryKeys.all,
      })
    }, REFRESH_DELAY_MS)
  }
  const subscription = client.subscribe(event => {
    if (SNAPSHOT_EVENTS.has(event.event)) refresh()
    const conversationEvent = toConversationEvent(event)
    if (!conversationEvent) return
    const { agentId, conversationId } = conversationEvent.payload
    // Only the companion's Studio conversation is kept here
    if (agentId !== STUDIO_AGENT_ID) return
    if (!useAgentStore.getState().conversations[conversationId]) return
    const applied = useAgentStore
      .getState()
      .applyConversationEvent(conversationEvent)
    if (!applied) void loadStudioConversation(conversationId)
  })
  return () => {
    clearTimeout(timer)
    subscription.unsubscribe()
  }
}

export function useCompanionSnapshot() {
  return useQuery({
    queryKey: companionQueryKeys.snapshot(),
    queryFn: async (): Promise<CompanionSnapshot> => {
      const result = await client.getCompanionSnapshot(localDayStart())
      if (result.status === 'error') throw new Error(result.error)
      return result.data
    },
    // Events keep it fresh
    staleTime: 60_000,
  })
}

interface PendingMessage {
  text: string
  createdAt: number
}

export interface UseCompanion {
  snapshot: CompanionSnapshot | undefined
  snapshotLoading: boolean
  snapshotError: boolean
  messages: UiMessage[]
  /** A message is being sent or its reply is streaming */
  busy: boolean
  sendError: string | null
  send: (text: string) => Promise<boolean>
  /** The Request the conversation is about (the scope chip) */
  scopeRequestId: string | null
  setScope: (requestId: string | null) => void
  pinned: boolean
  setPinned: (pinned: boolean) => Promise<void>
  notify: boolean
  setNotify: (notify: boolean) => Promise<void>
  /** "Notify me when requests come back" */
  notifyResults: boolean
}

const NO_MESSAGES: UiMessage[] = []

/** Everything the companion window shows. Call once per window. */
export function useCompanion(): UseCompanion {
  const queryClient = useQueryClient()
  const snapshotQuery = useCompanionSnapshot()
  const snapshot = snapshotQuery.data
  const [pending, setPending] = useState<PendingMessage | null>(null)
  const [sendError, setSendError] = useState<string | null>(null)
  const [pinnedOverride, setPinnedOverride] = useState<boolean | null>(null)
  const [notifyOverride, setNotifyOverride] = useState<boolean | null>(null)
  // `undefined`: the scope the backend remembers for the conversation
  const [scopeEdit, setScopeEdit] = useState<string | null | undefined>(
    undefined
  )
  const scopeRequestId =
    scopeEdit === undefined ? (snapshot?.scopeRequestId ?? null) : scopeEdit

  useEffect(() => subscribeCompanionEvents(queryClient), [queryClient])

  const studioConversationId = snapshot?.conversationId ?? null
  const conversation = useConversation(studioConversationId)

  useEffect(() => {
    if (studioConversationId) void loadStudioConversation(studioConversationId)
  }, [studioConversationId])

  const stored = conversation?.messages ?? NO_MESSAGES
  const messages: UiMessage[] = pending
    ? [
        ...stored,
        {
          kind: 'user',
          id: 'companion-pending',
          runId: null,
          createdAt: pending.createdAt,
          content: pending.text,
          attachments: [],
          pending: true,
        },
      ]
    : stored

  const send = async (text: string): Promise<boolean> => {
    const trimmed = text.trim()
    if (!trimmed || pending) return false
    setSendError(null)
    setPending({ text: trimmed, createdAt: Date.now() })
    try {
      const result = await client.sendCompanionMessage(
        trimmed,
        studioConversationId,
        scopeRequestId
      )
      if (result.status === 'error') {
        logger.warn('Companion: send failed', { error: result.error })
        setSendError(result.error)
        return false
      }
      // The first message creates the conversation: learn its ID, then
      // catch up on what was persisted (events stream the rest). An
      // "@Agent" message went to that agent: nothing to load here.
      await queryClient.refetchQueries({
        queryKey: companionQueryKeys.snapshot(),
      })
      const id =
        queryClient.getQueryData<CompanionSnapshot>(
          companionQueryKeys.snapshot()
        )?.conversationId ?? null
      if (id) await loadStudioConversation(id)
      return true
    } finally {
      setPending(null)
    }
  }

  const setNotify = async (next: boolean) => {
    setNotifyOverride(next)
    const result = await client.setCompanionNotify(next)
    if (result.status === 'error') {
      logger.warn('Companion: notify preference failed', {
        error: result.error,
      })
    } else {
      await queryClient.invalidateQueries({
        queryKey: companionQueryKeys.snapshot(),
      })
    }
    setNotifyOverride(null)
  }

  const setPinned = async (next: boolean) => {
    setPinnedOverride(next)
    const result = await client.setCompanionPinned(next)
    if (result.status === 'error') {
      logger.warn('Companion: pin failed', { error: result.error })
    } else {
      await queryClient.invalidateQueries({
        queryKey: companionQueryKeys.snapshot(),
      })
    }
    setPinnedOverride(null)
  }

  const run = conversation?.currentRun
  const busy =
    pending !== null ||
    run?.status === 'running' ||
    run?.status === 'awaitingApproval'

  return {
    snapshot,
    snapshotLoading: snapshotQuery.isPending,
    snapshotError: snapshotQuery.isError,
    messages,
    busy,
    sendError,
    send,
    scopeRequestId,
    // Sent with the next message; the backend remembers it
    setScope: setScopeEdit,
    pinned: pinnedOverride ?? snapshot?.pinned ?? false,
    setPinned,
    notify: notifyOverride ?? snapshot?.notify ?? true,
    setNotify,
    notifyResults: snapshot?.notifyResults ?? true,
  }
}
