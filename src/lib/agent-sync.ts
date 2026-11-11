/**
 * Keeps the agent store in sync with Rust: snapshots on demand, events as
 * deltas. Listeners are registered once at app boot (not per view), which
 * is what lets agents run in the background while the user navigates.
 *
 * Race-free loading: listeners register *before* snapshots are fetched, and
 * events for a conversation that is mid-load are buffered and replayed on
 * top of the snapshot. Reducers are idempotent, so replaying an event the
 * snapshot already contains is harmless.
 */

import { agentPoliciesQueryKeys } from '@/hooks/useAgentPolicies'
import { auditQueryKeys } from '@/hooks/useAuditLog'
import { memberQueryKeys } from '@/hooks/useNetwork'
import { policiesQueryKeys } from '@/hooks/usePolicies'
import { runPlanQueryKeys } from '@/hooks/useRunPlan'
import { runsQueryKeys } from '@/hooks/useRuns'
import { sharedQueryKeys } from '@/hooks/useSharedAgents'
import i18n from '@/i18n/config'
import { logger } from '@/lib/logger'
import { applyLiveChange } from '@/lib/live-changes'
import { notify } from '@/lib/notifications'
import { queryClient } from '@/lib/query-client'
import { toastError } from '@/lib/toast-error'
import { client, type NandiEvent } from '@/lib/client'
import type { AgentQueueChanged } from '@/lib/tauri-bindings'
import {
  isConversationEvent,
  type ConversationEvent,
  type DashboardEvent,
  type StoreEvent,
} from '@/lib/store-reducers'
import { useAgentStore } from '@/store/agent-store'
import { installKey, useMarketplaceStore } from '@/store/marketplace-store'

/** Debounce for event-driven dashboard/Studio-list refreshes */
const REFRESH_DELAY_MS = 250
const STUDIO_AGENT_ID = '@studio'

// In-flight loads, and events that arrived during them
const loadingConversations = new Map<string, ConversationEvent[]>()
// Agents whose snapshot is loading, with the latest queue event meanwhile
const loadingQueues = new Map<string, AgentQueueChanged | null>()
let dashboardBuffer: DashboardEvent[] | null = null
const timers = new Map<string, ReturnType<typeof setTimeout>>()

function debounce(key: string, fn: () => void) {
  clearTimeout(timers.get(key))
  timers.set(
    key,
    setTimeout(() => {
      timers.delete(key)
      fn()
    }, REFRESH_DELAY_MS)
  )
}

/** Start of today in local time (the dashboard's "today") */
export const localMidnight = () => {
  const date = new Date()
  date.setHours(0, 0, 0, 0)
  return date.getTime()
}

// ---------------------------------------------------------------------------
// Loaders
// ---------------------------------------------------------------------------

/** (Re)loads an agent snapshot, replaying events that arrived meanwhile. */
export async function loadAgent(agentId: string): Promise<void> {
  const conversationId =
    useAgentStore.getState().agents[agentId]?.conversationId ??
    `agent:${agentId}`
  if (loadingConversations.has(conversationId)) return
  loadingConversations.set(conversationId, [])
  loadingQueues.set(agentId, null)
  try {
    const result = await client.getAgentSnapshot(agentId)
    if (result.status === 'error') {
      logger.warn('Failed to load agent snapshot', {
        agentId,
        error: result.error,
      })
      return
    }
    useAgentStore.getState().setAgentSnapshot(result.data, agentId)
  } finally {
    // A queue event is the whole state: the latest one beats the snapshot
    const latestQueue = loadingQueues.get(agentId)
    loadingQueues.delete(agentId)
    if (latestQueue) applyQueueChanged(latestQueue)
    replay(conversationId)
  }
}

/** (Re)loads a Studio conversation's messages. */
export async function loadConversation(conversationId: string): Promise<void> {
  if (loadingConversations.has(conversationId)) return
  loadingConversations.set(conversationId, [])
  try {
    const result = await client.getConversation(conversationId)
    if (result.status === 'error') {
      logger.warn('Failed to load conversation', {
        conversationId,
        error: result.error,
      })
      return
    }
    useAgentStore.getState().setConversation(result.data)
  } finally {
    replay(conversationId)
  }
}

function replay(conversationId: string) {
  const buffered = loadingConversations.get(conversationId) ?? []
  loadingConversations.delete(conversationId)
  for (const event of buffered) applyConversationEvent(event)
}

export async function loadDashboard(): Promise<void> {
  if (dashboardBuffer) return
  dashboardBuffer = []
  try {
    const result = await client.getDashboardSnapshot(localMidnight())
    if (result.status === 'error') {
      logger.warn('Failed to load dashboard', { error: result.error })
      return
    }
    useAgentStore.getState().setDashboard(result.data)
  } finally {
    const buffered = dashboardBuffer ?? []
    dashboardBuffer = null
    for (const event of buffered)
      useAgentStore.getState().applyDashboardEvent(event)
  }
}

export async function loadStudioConversations(): Promise<void> {
  const result = await client.getStudioConversations()
  if (result.status === 'error') {
    logger.warn('Failed to load Studio conversations', { error: result.error })
    return
  }
  useAgentStore.getState().setStudio(result.data)
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

function applyConversationEvent(event: ConversationEvent) {
  const { agentId, conversationId } = event.payload
  const buffer = loadingConversations.get(conversationId)
  if (buffer) {
    buffer.push(event)
    return
  }

  const applied = useAgentStore.getState().applyConversationEvent(event)
  if (!applied) {
    logger.info('Event could not be applied; reloading snapshot', {
      type: event.type,
      conversationId,
    })
    if (agentId === STUDIO_AGENT_ID) void loadConversation(conversationId)
    else void loadAgent(agentId)
  }
}

function applyQueueChanged(payload: AgentQueueChanged) {
  if (loadingQueues.has(payload.agentId)) {
    loadingQueues.set(payload.agentId, payload)
    return
  }
  useAgentStore
    .getState()
    .setQueue(payload.agentId, payload.queue, payload.current)
}

/** Entry point for every backend event (exported for tests). */
export function handleEvent(event: StoreEvent) {
  if (!isConversationEvent(event)) {
    if (dashboardBuffer) dashboardBuffer.push(event)
    else useAgentStore.getState().applyDashboardEvent(event)
    if (event.type === 'activity') refreshRunHistory()
    return
  }

  applyConversationEvent(event)

  // Stats, health, and outputs are aggregates: refresh them from the DB
  // when a run starts or ends (event-driven, not polling)
  const { agentId } = event.payload
  if (event.type === 'turnDone' || event.type === 'run') {
    if (agentId === STUDIO_AGENT_ID) {
      debounce('studio', () => void loadStudioConversations())
    } else {
      debounce('dashboard', () => void loadDashboard())
      refreshRunHistory()
    }
  }
  // Run Detail follows its run live: every step, not just the end
  const runId = runIdOf(event)
  if (runId) {
    const refresh = () =>
      void queryClient.invalidateQueries({ queryKey: ['runDetail', runId] })
    if (event.type === 'turnDone') refresh()
    else debounce(`runDetail:${runId}`, refresh)
  }
}

/** The run a conversation event belongs to, when it names one */
function runIdOf(event: StoreEvent): string | null {
  switch (event.type) {
    case 'run':
      return event.payload.run.id
    case 'toolCall':
    case 'toolResult':
    case 'approvalRequest':
    case 'approvalResolved':
    case 'turnDone':
      return event.payload.runId
    default:
      return null
  }
}

/**
 * The global runs view and the LLM audit log: a run started, moved, or
 * ended (its model calls are audited as it goes). Debounced: a busy agent
 * emits many run updates.
 */
function refreshRunHistory() {
  debounce('runHistory', () => {
    void queryClient.invalidateQueries({ queryKey: runsQueryKeys.all })
    void queryClient.invalidateQueries({
      queryKey: auditQueryKeys.kind('llm'),
    })
  })
}

/** Maps a wire event onto the store's event types. */
function toStoreEvent(event: NandiEvent): StoreEvent | null {
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
    case 'dashboard://attention':
      return { type: 'attention', payload: event.payload }
    case 'dashboard://activity':
      return { type: 'activity', payload: event.payload }
    default:
      return null
  }
}

/** Entry point for every wire event (exported for tests). */
export function handleNandiEvent(event: NandiEvent) {
  switch (event.event) {
    case 'agent://agents_changed':
      // Created or deleted (possibly by another client)
      void queryClient.invalidateQueries({ queryKey: ['agents'] })
      void loadAgent(event.payload.agentId)
      debounce('dashboard', () => void loadDashboard())
      return
    case 'agent://warning':
      logger.warn('Agent warning', event.payload)
      useAgentStore
        .getState()
        .setAgentWarning(event.payload.agentId, event.payload.message)
      return
    case 'agent://queue_changed':
      applyQueueChanged(event.payload)
      return
    // A run's checklist card: the event carries the whole plan
    case 'run://plan_updated':
      queryClient.setQueryData(
        runPlanQueryKeys.plan(event.payload.runId),
        event.payload.plan
      )
      return
    // Run Detail: a stopped turn, or a CLI runtime's own decision
    case 'agent://turn_aborted':
      void queryClient.invalidateQueries({
        queryKey: ['runDetail', event.payload.runId],
      })
      return
    case 'harness://permission_request': {
      const runId = (event.payload as { runId?: unknown } | null)?.runId
      if (typeof runId === 'string') {
        debounce(`runDetail:${runId}`, () => {
          void queryClient.invalidateQueries({
            queryKey: ['runDetail', runId],
          })
        })
      }
      return
    }
    case 'nandi://resync':
      logger.info('Resyncing from snapshots', event.payload)
      void resyncAll()
      return
    case 'workflow://changed':
      void queryClient.invalidateQueries({ queryKey: ['workflows'] })
      void queryClient.invalidateQueries({ queryKey: ['workflow'] })
      return
    case 'trigger://changed':
      void queryClient.invalidateQueries({ queryKey: ['triggers'] })
      void queryClient.invalidateQueries({ queryKey: ['trigger'] })
      debounce('dashboard', () => void loadDashboard())
      return
    case 'trigger://fired':
    case 'trigger://rate_limited':
    case 'trigger://error': {
      // Last-fired times, fire history, and dashboard counts all move
      const { triggerId } = event.payload
      void queryClient.invalidateQueries({ queryKey: ['triggers'] })
      void queryClient.invalidateQueries({ queryKey: ['trigger', triggerId] })
      void queryClient.invalidateQueries({
        queryKey: ['triggerFires', triggerId],
      })
      void queryClient.invalidateQueries({
        queryKey: auditQueryKeys.kind('triggers'),
      })
      debounce('dashboard', () => void loadDashboard())
      return
    }
    // Wave 1: minimal invalidations; the owning views refetch what they show
    case 'repo://file_changed':
      // A gold-copy file changed: git status, and the entity as stored
      void queryClient.invalidateQueries({ queryKey: ['git'] })
      void queryClient.invalidateQueries({
        queryKey: ['repoEntity', event.payload.kind, event.payload.id],
      })
      // The agent definition's keys (useAgentEditor.ts)
      void queryClient.invalidateQueries({ queryKey: ['git-status'] })
      if (event.payload.kind === 'agent') {
        void queryClient.invalidateQueries({
          queryKey: ['agent-definition', event.payload.id],
        })
        // What its policies change depends on what skill.md asks for
        void queryClient.invalidateQueries({
          queryKey: agentPoliciesQueryKeys.agent(event.payload.id),
        })
      }
      // A policy may cover any agent
      if (event.payload.kind === 'policy') {
        void queryClient.invalidateQueries({
          queryKey: agentPoliciesQueryKeys.all,
        })
        void queryClient.invalidateQueries({ queryKey: policiesQueryKeys.all })
      }
      return
    case 'studio://artifact_saved':
      // A policy may cover any agent
      if (event.payload.result.artifact.kind === 'policy') {
        void queryClient.invalidateQueries({
          queryKey: agentPoliciesQueryKeys.all,
        })
        void queryClient.invalidateQueries({ queryKey: policiesQueryKeys.all })
      }
      return
    case 'repo://validation_failed':
      logger.warn('Repo file failed validation', event.payload)
      return
    case 'live://changed':
      // A file the app shows changed on disk, whoever changed it
      applyLiveChange(event.payload)
      return
    case 'pilot://changed':
      queryClient.setQueryData(
        ['pilotStatus', event.payload.pilotId],
        event.payload.status
      )
      void queryClient.invalidateQueries({ queryKey: ['pilots'] })
      void queryClient.invalidateQueries({
        queryKey: auditQueryKeys.kind('pilots'),
      })
      return
    case 'connector://changed':
      queryClient.setQueryData(
        ['connectorStatus', event.payload.connectorId],
        event.payload.status
      )
      void queryClient.invalidateQueries({ queryKey: ['connectors'] })
      return
    case 'connectors://changed':
      void queryClient.invalidateQueries({ queryKey: ['connectors'] })
      void queryClient.invalidateQueries({ queryKey: ['connectorStatus'] })
      return
    case 'marketplace://progress': {
      const { kind, id, receivedObjects, totalObjects } = event.payload
      useMarketplaceStore.getState().setProgress(installKey(kind, id), {
        received: receivedObjects,
        total: totalObjects,
      })
      return
    }
    case 'marketplace://installed':
    case 'marketplace://uninstalled':
    case 'marketplace://forked':
    case 'marketplace://update_applied':
      // Install state shows on every catalog entry; the agent list itself
      // follows from the repo re-scan (`agent://agents_changed`)
      void queryClient.invalidateQueries({ queryKey: ['marketplace'] })
      return
    case 'marketplace://update_available':
      return
    case 'marketplace://error':
      logger.warn('Marketplace action failed', event.payload)
      return
    // Wave 2A: each track's data lives under its own key prefix; the
    // shared views (`['shared']`) cut across all of them
    case 'relay://status':
      void queryClient.invalidateQueries({ queryKey: ['relay'] })
      return
    case 'relay://agent_online':
    case 'relay://agent_offline':
      void queryClient.invalidateQueries({ queryKey: ['relay'] })
      void queryClient.invalidateQueries({ queryKey: ['capabilities'] })
      void queryClient.invalidateQueries({ queryKey: sharedQueryKeys.withMe })
      void queryClient.invalidateQueries({
        queryKey: sharedQueryKeys.capabilitiesAll,
      })
      return
    case 'relay://seal_refused':
      // Never dropped silently: the user hears that something was refused
      logger.warn('Refused a sealed message', event.payload)
      toastError(event.payload.error)
      void queryClient.invalidateQueries({ queryKey: ['delegation'] })
      return
    case 'network://member_key_changed':
      // Their profile shows the change to check and accept
      void queryClient.invalidateQueries({
        queryKey: memberQueryKeys.byInstance(event.payload.instanceId),
      })
      void queryClient.invalidateQueries({ queryKey: memberQueryKeys.all })
      return
    case 'team://changed':
      void queryClient.invalidateQueries({ queryKey: ['teams'] })
      void queryClient.invalidateQueries({ queryKey: sharedQueryKeys.all })
      return
    case 'delegation://task_started':
    case 'delegation://task_completed':
    case 'delegation://task_failed':
      void queryClient.invalidateQueries({ queryKey: ['delegation'] })
      void queryClient.invalidateQueries({ queryKey: sharedQueryKeys.all })
      return
    case 'workflow://step_started':
    case 'workflow://step_awaiting_approval':
    case 'workflow://step_done':
    case 'workflow://done':
    case 'workflow://failed':
      // Run state lives in the workflow snapshot (definition + runs)
      void queryClient.invalidateQueries({
        queryKey: ['workflow', event.payload.workflowId],
      })
      return
    case 'workflow://manual_step_ready':
      // Possibly days into a run: say so outside the app too
      void notify(
        i18n.t('work.manual.readyTitle', {
          workflow: event.payload.workflowName || event.payload.workflowId,
        }),
        event.payload.instructions,
        { native: true }
      )
      void queryClient.invalidateQueries({
        queryKey: ['workflow', event.payload.workflowId],
      })
      debounce('dashboard', () => void loadDashboard())
      return
    case 'workflow://manual_step_completed':
      // The dashboard's "Needs your attention" listed it
      void queryClient.invalidateQueries({
        queryKey: ['workflow', event.payload.workflowId],
      })
      debounce('dashboard', () => void loadDashboard())
      return
    case 'task://reassigned':
      // The requester's side: their task moved to someone else
      if (event.payload.requester) {
        void notify(
          i18n.t('delegationThread.item.reassigned', {
            to: event.payload.toName ?? event.payload.toInstance,
            by: event.payload.by,
          })
        )
      }
      return
  }
  const storeEvent = toStoreEvent(event)
  if (storeEvent) handleEvent(storeEvent)
}

/**
 * Subscribes to the core's events (once), then loads the initial
 * snapshots. Returns a cleanup function.
 */
export function startAgentSync(): () => void {
  const subscription = client.subscribe(handleNandiEvent)
  void subscription.ready.then(() => bootSnapshots())
  return subscription.unsubscribe
}

async function bootSnapshots() {
  const agents = await client.listAgents()
  const agentIds = agents.status === 'ok' ? agents.data.map(a => a.id) : []
  await Promise.all([
    loadDashboard(),
    loadStudioConversations(),
    ...agentIds.map(loadAgent),
  ])
  logger.info('Agent store ready', { agents: agentIds.length })
}

/** Reloads everything we hold (events may have been missed). */
async function resyncAll() {
  const conversations = Object.keys(useAgentStore.getState().conversations)
  const studioConversations = conversations.filter(
    id => !id.startsWith('agent:')
  )
  // Every query: a missed event could have changed any of them
  void queryClient.invalidateQueries()
  await Promise.all([
    bootSnapshots(),
    ...studioConversations.map(loadConversation),
  ])
}
