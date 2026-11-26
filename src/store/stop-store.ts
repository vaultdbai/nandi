import { create } from 'zustand'
import { devtools } from 'zustand/middleware'

/** "Close the browser too?" after a stop (`services::stop`) */
export interface BrowserQuestion {
  id: string
  /** When it closes on its own (epoch ms) */
  until: number
}

interface StopState {
  question: BrowserQuestion | null
  ask: (question: BrowserQuestion) => void
  clear: () => void
}

/** The browser question a stop is waiting on, shown in the open chat */
export const useStopStore = create<StopState>()(
  devtools(
    set => ({
      question: null,
      ask: question => set({ question }, undefined, 'ask'),
      clear: () => set({ question: null }, undefined, 'clear'),
    }),
    { name: 'stop-store' }
  )
)
