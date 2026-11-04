import { create } from 'zustand'
import { devtools } from 'zustand/middleware'
import type { ArtifactRef, EditPreview } from '@/lib/bindings'

/** Studio's right-pane tabs; `tool` is the read-only tool catalog */
export type ArtifactTab =
  'capability' | 'agent' | 'workflow' | 'policy' | 'tool'

export const ARTIFACT_TABS: ArtifactTab[] = [
  'capability',
  'agent',
  'workflow',
  'policy',
  'tool',
]

interface StudioState {
  tab: ArtifactTab
  /** The artifact open in each tab (an ID; tools by tool ID) */
  selected: Partial<Record<ArtifactTab, string>>
  /** An approved edit waiting to be loaded into the editor */
  pendingEdit: EditPreview | null
  setTab: (tab: ArtifactTab) => void
  select: (tab: ArtifactTab, id: string | null) => void
  /** Opens the edit's artifact and queues its new text for the editor */
  applyEdit: (preview: EditPreview) => void
  /** The approved edit for `artifact`, if one is waiting */
  pendingEditFor: (artifact: ArtifactRef) => EditPreview | null
  clearPendingEdit: (artifact: ArtifactRef) => void
}

/** The tab an artifact opens in (NANDI.md opens under Policy) */
export function tabOf(artifact: ArtifactRef): ArtifactTab {
  return artifact.kind === 'gateway' ? 'policy' : artifact.kind
}

/** The artifact a tab's selection names */
export function artifactOf(tab: ArtifactTab, id: string): ArtifactRef | null {
  switch (tab) {
    case 'tool':
      return null
    case 'policy':
      return id === 'gateway' ? { kind: 'gateway' } : { kind: 'policy', id }
    default:
      return { kind: tab, id }
  }
}

const sameArtifact = (a: ArtifactRef, b: ArtifactRef) =>
  a.kind === b.kind &&
  (a.kind === 'gateway' || (b.kind !== 'gateway' && a.id === b.id))

export const useStudioStore = create<StudioState>()(
  devtools(
    (set, get) => ({
      tab: 'capability',
      selected: {},
      pendingEdit: null,
      setTab: tab => set({ tab }, undefined, 'setTab'),
      select: (tab, id) =>
        set(
          state => ({
            selected: { ...state.selected, [tab]: id ?? undefined },
          }),
          undefined,
          'select'
        ),
      applyEdit: preview => {
        const tab = tabOf(preview.artifact)
        const id =
          preview.artifact.kind === 'gateway' ? 'gateway' : preview.artifact.id
        set(
          state => ({
            tab,
            selected: { ...state.selected, [tab]: id },
            pendingEdit: preview,
          }),
          undefined,
          'applyEdit'
        )
      },
      pendingEditFor: artifact => {
        const pending = get().pendingEdit
        return pending && sameArtifact(pending.artifact, artifact)
          ? pending
          : null
      },
      clearPendingEdit: artifact => {
        if (get().pendingEditFor(artifact)) {
          set({ pendingEdit: null }, undefined, 'clearPendingEdit')
        }
      },
    }),
    { name: 'studio-store' }
  )
)
