import { create } from 'zustand'
import { devtools } from 'zustand/middleware'
import type { ExampleKind } from '@/components/nandi/examples-model'

/** Where the examples' memory is kept (this computer only) */
const STORAGE_KEY = 'nandi-examples'

interface Saved {
  tapped: ExampleKind[]
  lastShown: string[]
}

function load(): Saved {
  try {
    const saved = JSON.parse(
      localStorage.getItem(STORAGE_KEY) ?? '{}'
    ) as Partial<Saved>
    return {
      tapped: Array.isArray(saved.tapped) ? saved.tapped : [],
      lastShown: Array.isArray(saved.lastShown) ? saved.lastShown : [],
    }
  } catch {
    return { tapped: [], lastShown: [] }
  }
}

function save(saved: Saved) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(saved))
  } catch {
    // Private windows and blocked storage: the examples just rotate
  }
}

interface ExamplesState extends Saved {
  /** The person sent an example of this kind: it steps back */
  tap: (kind: ExampleKind) => void
  /** The ids a set just showed, for the next set to leave out */
  showed: (ids: string[]) => void
}

/** What the examples remember: kinds tapped, the last set shown */
export const useExamplesStore = create<ExamplesState>()(
  devtools(
    (set, get) => ({
      ...load(),
      tap: kind => {
        const tapped = get().tapped.includes(kind)
          ? get().tapped
          : [...get().tapped, kind]
        save({ tapped, lastShown: get().lastShown })
        set({ tapped }, undefined, 'tap')
      },
      showed: ids => {
        save({ tapped: get().tapped, lastShown: ids })
        set({ lastShown: ids }, undefined, 'showed')
      },
    }),
    { name: 'examples-store' }
  )
)
