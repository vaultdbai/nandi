import { create } from 'zustand'
import { devtools } from 'zustand/middleware'

/**
 * The Request pinned at the top of the companion's Running (Watch in
 * companion). The desktop window sets it from `companion://focus_request`;
 * the web build's in-window companion shares the main page's store.
 */
interface CompanionState {
  focusedRequestId: string | null
  focusRequest: (requestId: string) => void
  clearFocus: () => void
}

export const useCompanionStore = create<CompanionState>()(
  devtools(
    set => ({
      focusedRequestId: null,
      focusRequest: requestId =>
        set({ focusedRequestId: requestId }, undefined, 'focusRequest'),
      clearFocus: () =>
        set({ focusedRequestId: null }, undefined, 'clearFocus'),
    }),
    { name: 'companion-store' }
  )
)
