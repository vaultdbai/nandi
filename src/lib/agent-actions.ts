/**
 * User actions. Each calls the Nandi client (IPC or HTTP); state changes
 * arrive back as events. The only local state is the optimistic user message, which
 * Rust confirms (same ID) before `send_message` returns.
 */

import { toast } from 'sonner'
import i18n from '@/i18n/config'
import { logger } from '@/lib/logger'
import { client } from '@/lib/client'
import type {
  AttachmentInfo,
  Result,
  SendMessageResult,
} from '@/lib/tauri-bindings'
import {
  DEFAULT_AGENT,
  type AskTarget,
  type HandOverAnswer,
  type StopOutcome,
} from '@/lib/bindings'
import { toastPersonError } from '@/lib/toast-error'
import { loadAgent, loadConversation } from '@/lib/agent-sync'
import { useAgentStore } from '@/store/agent-store'
import { useNavigationStore } from '@/store/navigation-store'
import { useStopStore } from '@/store/stop-store'
import { queryClient } from '@/lib/query-client'
import { chatsQueryKey } from '@/hooks/useChats'

async function sendOptimistically(
  conversationId: string,
  text: string,
  attachments: AttachmentInfo[],
  send: (messageId: string) => Promise<Result<SendMessageResult, string>>
): Promise<SendMessageResult | null> {
  const store = useAgentStore.getState()
  const id = crypto.randomUUID()
  store.addPendingMessage(conversationId, {
    kind: 'user',
    id,
    runId: null,
    createdAt: Date.now(),
    content: text.trim(),
    attachments,
    asked: [],
  })

  const result = await send(id)
  if (result.status === 'error') {
    logger.warn('Send failed', { conversationId, error: result.error })
    store.removeMessage(conversationId, id)
    toast.error(result.error)
    return null
  }
  if (result.data.stopped) {
    // Just "stop": the run was stopped, nothing was sent
    store.removeMessage(conversationId, id)
    onStopped(result.data.stopped)
    return result.data
  }
  if (result.data.queued) {
    // It joins the conversation when its run starts; until then it's in
    // the agent's queue
    store.dropPendingMessage(conversationId, id)
    toast.info(i18n.t('agentConsole.queue.queuedToast'))
  } else {
    store.confirmMessage(conversationId, id)
  }
  return result.data
}

export async function sendAgentMessage(
  agentId: string,
  text: string,
  attachments: AttachmentInfo[] = []
): Promise<void> {
  const state = useAgentStore.getState()
  if (!state.agents[agentId]) await loadAgent(agentId)
  const conversationId =
    useAgentStore.getState().agents[agentId]?.conversationId ??
    `agent:${agentId}`

  await sendOptimistically(conversationId, text, attachments, messageId =>
    client.sendMessage(
      agentId,
      text,
      attachments.map(a => a.id),
      messageId
    )
  )
}

/**
 * A reply in a live run's chat: the next turn in that run's own
 * conversation, as its agent. `null` when it failed (a toast says why).
 */
export async function replyToRun(
  runId: string,
  conversationId: string,
  text: string,
  attachments: AttachmentInfo[] = []
): Promise<SendMessageResult | null> {
  return sendOptimistically(conversationId, text, attachments, messageId =>
    client.replyToRun(
      runId,
      text,
      attachments.map(a => a.id),
      messageId
    )
  )
}

export async function sendStudioMessage(
  conversationId: string,
  text: string,
  attachments: AttachmentInfo[] = []
): Promise<void> {
  if (!useAgentStore.getState().conversations[conversationId]) {
    await loadConversation(conversationId)
  }
  const sent = await sendOptimistically(
    conversationId,
    text,
    attachments,
    messageId =>
      client.sendStudioMessage(
        conversationId,
        text,
        attachments.map(a => a.id),
        messageId
      )
  )
  const title = sent?.conversationTitle
  if (title) {
    const store = useAgentStore.getState()
    const summary = store.studio?.find(c => c.id === conversationId)
    if (summary) store.upsertStudioConversation({ ...summary, title })
  }
}

/** A message to Nandi in an intake item's thread */
export async function sendIntakeMessage(
  conversationId: string,
  text: string
): Promise<void> {
  if (!useAgentStore.getState().conversations[conversationId]) {
    await loadConversation(conversationId)
  }
  await sendOptimistically(conversationId, text, [], messageId =>
    client.sendIntakeMessage(conversationId, text, messageId)
  )
}

/** The user's message in an Inbox item's Ask chat: a Nandi turn */
export async function sendInboxMessage(
  conversationId: string,
  text: string
): Promise<void> {
  if (!useAgentStore.getState().conversations[conversationId]) {
    await loadConversation(conversationId)
  }
  await sendOptimistically(conversationId, text, [], messageId =>
    client.sendInboxMessage(conversationId, text, messageId)
  )
}

/**
 * Opens the Nandi home's composer. Nothing is created until the first
 * message is sent (`startChat`).
 */
export function openNewChat(): void {
  useNavigationStore.getState().navigate({ kind: 'nandi' })
}

/**
 * Starts a new chat with an agent (Nandi by default) with this message:
 * its own conversation, opened on the Nandi home
 */
export async function startChat(
  agentId: string,
  text: string,
  attachments: AttachmentInfo[] = []
): Promise<string | null> {
  const result = await client.createChat(agentId)
  if (result.status === 'error') {
    toast.error(result.error)
    return null
  }
  const conversationId = result.data.id
  useAgentStore.getState().setConversation({
    conversationId,
    messages: [],
    currentRun: null,
  })
  useNavigationStore.getState().navigate({ kind: 'nandi', conversationId })
  await sendChatMessage(conversationId, text, attachments)
  return conversationId
}

/** The next message in one of the person's chats with an agent */
export async function sendChatMessage(
  conversationId: string,
  text: string,
  attachments: AttachmentInfo[] = []
): Promise<void> {
  await sendOptimistically(conversationId, text, attachments, messageId =>
    client.sendChatMessage(
      conversationId,
      text,
      attachments.map(a => a.id),
      messageId
    )
  )
  void queryClient.invalidateQueries({ queryKey: chatsQueryKey })
}

/** Creates a Studio conversation in the DB and opens it. */
export async function startStudioChat(): Promise<string | null> {
  const result = await client.createStudioConversation()
  if (result.status === 'error') {
    toast.error(result.error)
    return null
  }
  const store = useAgentStore.getState()
  store.upsertStudioConversation(result.data)
  store.setConversation({
    conversationId: result.data.id,
    messages: [],
    currentRun: null,
  })
  useNavigationStore
    .getState()
    .navigate({ kind: 'studio', conversationId: result.data.id })
  return result.data.id
}

/** Sends to an open Studio conversation, or starts one with this message. */
export async function sendToStudio(
  text: string,
  attachments: AttachmentInfo[],
  conversationId: string | null
): Promise<void> {
  const id = conversationId ?? (await startStudioChat())
  if (id) await sendStudioMessage(id, text, attachments)
}

/** Returns whether Rust accepted the decision */
export async function decideApproval(
  agentId: string,
  approvalId: string,
  decision: 'approved' | 'rejected'
): Promise<boolean> {
  const result = await client.approveAction(agentId, approvalId, decision)
  if (result.status === 'error') {
    toast.error(result.error)
    return false
  }
  return true
}

/** Answers a browser hand-over: done, skipped, or changed with the text */
export async function answerHandOver(
  approvalId: string,
  answer: HandOverAnswer
): Promise<boolean> {
  const result = await client.answerHandOver(approvalId, answer)
  if (result.status === 'error') {
    toast.error(result.error)
    return false
  }
  return true
}

/**
 * `@Sam …` or `@Finance …`: the ask to a person or group, kept in the chat
 * (`conversationId`, or a new Nandi chat) with a card for each
 */
export async function askPeople(
  conversationId: string | null,
  target: AskTarget,
  text: string
): Promise<void> {
  let chat = conversationId
  if (!chat) {
    const created = await client.createChat(DEFAULT_AGENT)
    if (created.status === 'error') {
      toast.error(created.error)
      return
    }
    chat = created.data.id
    useNavigationStore
      .getState()
      .navigate({ kind: 'nandi', conversationId: chat })
  }
  const result = await client.askPeopleInChat(chat, target, text)
  if (result.status === 'error') {
    toastPersonError(result.error, error => toast.error(String(error)))
    return
  }
  await loadConversation(chat)
  void queryClient.invalidateQueries({ queryKey: chatsQueryKey })
}

/** How long the browser question waits (`stop::BROWSER_WAIT`) */
const BROWSER_WAIT_MS = 120_000

/** Says what a stop did, and asks about the browser if it's open */
export function onStopped(outcome: StopOutcome): void {
  toast.success(
    outcome.dropped.length > 0
      ? i18n.t('stop.stoppedDropped', { count: outcome.dropped.length })
      : i18n.t('stop.stopped')
  )
  if (outcome.browserQuestion) {
    useStopStore.getState().ask({
      id: outcome.browserQuestion,
      until: Date.now() + BROWSER_WAIT_MS,
    })
  }
}

/** Stops a run and everything it started (`services::stop`) */
export async function stopRun(runId: string): Promise<void> {
  const result = await client.stopRun(runId)
  if (result.status === 'error') {
    toast.error(result.error)
    return
  }
  onStopped(result.data)
}

/** The answer to "Close the browser too?" */
export async function answerBrowserQuestion(
  questionId: string,
  close: boolean
): Promise<void> {
  useStopStore.getState().clear()
  const result = await client.answerBrowserQuestion(questionId, close)
  if (result.status === 'error') toast.error(result.error)
}

export async function cancelTurn(
  agentId: string,
  runId: string
): Promise<void> {
  const result = await client.cancelTurn(agentId, runId)
  if (result.status === 'error') toast.error(result.error)
}

/** Removes a run from its agent's queue */
export async function cancelQueuedRun(runId: string): Promise<void> {
  const result = await client.cancelQueuedRun(runId)
  if (result.status === 'error') toast.error(result.error)
}

/** Moves a queued run; lower priorities run sooner */
export async function reprioritizeQueuedRun(
  runId: string,
  priority: number
): Promise<void> {
  const result = await client.reprioritizeQueuedRun(runId, priority)
  if (result.status === 'error') toast.error(result.error)
}

export type ContainerAction = 'start' | 'stop' | 'restart'

/** Starts, stops or restarts the agent's container; returns success. */
export async function controlContainer(
  agentId: string,
  action: ContainerAction
): Promise<boolean> {
  const result = await {
    start: client.startAgent,
    stop: client.stopAgent,
    restart: client.restartAgent,
  }[action](agentId)
  // The status changes arrive as `agent://agents_changed`; reload anyway
  // in case this client missed them
  await loadAgent(agentId)
  if (result.status === 'error') {
    logger.warn('Container action failed', {
      agentId,
      action,
      error: result.error,
    })
    toast.error(result.error)
    return false
  }
  if (action !== 'stop') useAgentStore.getState().setAgentWarning(agentId, null)
  return true
}
