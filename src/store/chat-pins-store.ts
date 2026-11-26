import { create } from 'zustand'
import { devtools } from 'zustand/middleware'

/** Where pinned chats are remembered (this computer only) */
const STORAGE_KEY = 'nandi-pinned-chats'

function load(): string[] {
  try {
    const saved: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]')
    return Array.isArray(saved)
      ? saved.filter((id): id is string => typeof id === 'string')
      : []
  } catch {
    return []
  }
}

function save(ids: string[]) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(ids))
  } catch {
    // Private windows and blocked storage: pins last until the app closes
  }
}

interface ChatPinsState {
  /** Conversation IDs of the chats kept under Recent */
  pinned: string[]
  toggle: (conversationId: string) => void
}

/** Chats pinned to stay under Recent on the Nandi home */
export const useChatPinsStore = create<ChatPinsState>()(
  devtools(
    (set, get) => ({
      pinned: load(),
      toggle: conversationId => {
        const current = get().pinned
        const pinned = current.includes(conversationId)
          ? current.filter(id => id !== conversationId)
          : [...current, conversationId]
        save(pinned)
        set({ pinned }, undefined, 'toggle')
      },
    }),
    { name: 'chat-pins-store' }
  )
)
