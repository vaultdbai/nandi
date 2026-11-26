/**
 * The Nandi home: one place to talk. Which chats it shows under Recent
 * (pinned ones stay, then the newest, ten in all; the rest are older) and
 * how a chat is named. Pure; `NandiHomeView` renders it.
 */
import type { AgentInfo, ChatSummary } from '@/lib/bindings'
import type { LiveLabel } from '@/components/live-runs/live-run-model'

/** Chats under Recent; the rest are in History */
export const RECENT_LIMIT = 10

/** An agent's long-lived chat (`agent:<id>`), from before chats were separate */
export function isLongChat(chat: Pick<ChatSummary, 'id' | 'agentId'>): boolean {
  return chat.id === `agent:${chat.agentId}`
}

/** Its title; untitled, "Earlier chats with Nandi" or "New chat" */
export function chatLabel(
  chat: Pick<ChatSummary, 'id' | 'agentId' | 'title'>,
  agents: readonly AgentInfo[]
): LiveLabel {
  if (chat.title.trim()) return { text: chat.title.trim() }
  if (isLongChat(chat)) {
    const name = agents.find(a => a.id === chat.agentId)?.name || chat.agentId
    return { key: 'nandiHome.longChat', values: { name } }
  }
  return { key: 'nandiHome.untitled' }
}

/**
 * Recent: the pinned chats first, then the newest, `limit` in all (pinned
 * ones always stay); older: the rest, newest first
 */
export function recentChats(
  chats: readonly ChatSummary[],
  pinned: readonly string[],
  limit = RECENT_LIMIT
): { recent: ChatSummary[]; older: ChatSummary[] } {
  const newest = [...chats].sort((a, b) => b.updatedAt - a.updatedAt)
  const pins = newest.filter(chat => pinned.includes(chat.id))
  const rest = newest.filter(chat => !pinned.includes(chat.id))
  const room = Math.max(0, limit - pins.length)
  return { recent: [...pins, ...rest.slice(0, room)], older: rest.slice(room) }
}

/** The agent a chat is with: its row's agent, or the one in its ID */
export function chatAgentId(
  conversationId: string,
  chats: readonly ChatSummary[]
): string | null {
  const listed = chats.find(chat => chat.id === conversationId)
  if (listed) return listed.agentId
  return conversationId.startsWith('agent:')
    ? conversationId.slice('agent:'.length)
    : null
}
