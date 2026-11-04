import { create } from 'zustand'
import { devtools } from 'zustand/middleware'

/** Download progress of one install or update */
export interface FetchProgress {
  received: number
  total: number
}

/** `pack:research-pack`, `agent:arxiv-researcher` */
export const installKey = (kind: string, id: string) => `${kind}:${id}`

interface MarketplaceState {
  /** Live progress from `marketplace://progress`, by `installKey` */
  progress: Record<string, FetchProgress>
  /** Installs and updates in flight, by `installKey` */
  busy: Record<string, true>

  setBusy: (key: string, busy: boolean) => void
  setProgress: (key: string, progress: FetchProgress) => void
  clearProgress: (key: string) => void
}

export const useMarketplaceStore = create<MarketplaceState>()(
  devtools(
    set => ({
      progress: {},
      busy: {},

      setBusy: (key, busy) =>
        set(
          state => {
            if (busy) return { busy: { ...state.busy, [key]: true } }
            if (!(key in state.busy)) return state
            const { [key]: _done, ...rest } = state.busy
            return { busy: rest }
          },
          undefined,
          'setBusy'
        ),

      setProgress: (key, progress) =>
        set(
          state => ({ progress: { ...state.progress, [key]: progress } }),
          undefined,
          'setProgress'
        ),

      clearProgress: key =>
        set(
          state => {
            if (!(key in state.progress)) return state
            const { [key]: _removed, ...rest } = state.progress
            return { progress: rest }
          },
          undefined,
          'clearProgress'
        ),
    }),
    { name: 'marketplace-store' }
  )
)
