/**
 * Studio's artifact editor (TanStack Query): load an artifact, save it
 * (commit, optionally push), preview an edit, and the push targets. Saves
 * refresh the loaded artifact; `studio://artifact_saved` from elsewhere
 * (another window, the web client) does too.
 */
import { useEffect } from 'react'
import { useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { ArtifactRef, ArtifactSave } from '@/lib/bindings'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

/** Everything Studio caches lives under `['studio']` */
export const studioKeys = {
  all: ['studio'] as const,
  artifact: (artifact: ArtifactRef) =>
    [
      'studio',
      'artifact',
      artifact.kind,
      artifact.kind === 'gateway' ? '' : artifact.id,
    ] as const,
  pushTargets: ['studio', 'push-targets'] as const,
}

/** The artifact's file and parsed view (idle while `null`) */
export function useArtifact(artifact: ArtifactRef | null) {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (event.event === 'studio://artifact_saved') {
        void queryClient.invalidateQueries({
          queryKey: studioKeys.artifact(event.payload.result.artifact),
        })
      }
    })
    return subscription.unsubscribe
  }, [queryClient])

  return useClientQuery(
    artifact ? studioKeys.artifact(artifact) : studioKeys.all,
    () => client.loadArtifact(artifact as ArtifactRef),
    { enabled: artifact !== null }
  )
}

export function useSaveArtifact() {
  const queryClient = useQueryClient()
  return useClientMutation((save: ArtifactSave) => client.saveArtifact(save), {
    onSuccess: result =>
      queryClient.invalidateQueries({
        queryKey: studioKeys.artifact(result.artifact),
      }),
  })
}

/**
 * Whether the draft would save, checked as the user types (idle while
 * `content` is `null`); the previous answer stays while the next loads
 */
export function useArtifactValidation(
  artifact: ArtifactRef,
  content: string | null
) {
  return useClientQuery(
    [...studioKeys.artifact(artifact), 'validate', content],
    () => client.validateArtifact(artifact, content ?? ''),
    { enabled: content !== null, placeholderData: previous => previous }
  )
}

/** Local commit, push to `<prefix>/<slug>`, push + pull request */
export function usePushTargets() {
  return useClientQuery(studioKeys.pushTargets, () => client.listPushTargets())
}
