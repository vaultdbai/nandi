import { create } from 'zustand'
import { devtools } from 'zustand/middleware'

/** Where the open branches are remembered (this computer only) */
const STORAGE_KEY = 'nav-tree-expanded'

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
    // Private windows and blocked storage: the tree just starts closed
  }
}

interface NavTreeState {
  /** Ids of the open branches */
  expanded: string[]
  toggle: (id: string) => void
  setExpanded: (id: string, open: boolean) => void
  /** Opens every id (a view's ancestors), keeping the rest as they are */
  reveal: (ids: readonly string[]) => void
}

export const useNavTreeStore = create<NavTreeState>()(
  devtools(
    (set, get) => {
      const update = (expanded: string[], action: string) => {
        save(expanded)
        set({ expanded }, undefined, action)
      }
      return {
        expanded: load(),
        toggle: id => get().setExpanded(id, !get().expanded.includes(id)),
        setExpanded: (id, open) => {
          const rest = get().expanded.filter(e => e !== id)
          update(open ? [...rest, id] : rest, 'setExpanded')
        },
        reveal: ids => {
          const current = get().expanded
          const missing = ids.filter(id => !current.includes(id))
          if (missing.length > 0) update([...current, ...missing], 'reveal')
        },
      }
    },
    { name: 'nav-tree-store' }
  )
)
