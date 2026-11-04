/**
 * A sandbox image: whether it's here, and a download's or build's progress,
 * kept live by `images://build` events. Both run in the core, so leaving
 * the page doesn't stop them; coming back shows the log so far.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { ImageStatus, SandboxImage } from '@/lib/bindings'
import { harnessQueryKeys } from '@/hooks/useHarnesses'

export const imageQueryKeys = {
  status: (image: SandboxImage) => ['images', image] as const,
}

/** Lines of output shown */
const LOG_LINES = 300

export function useImageStatus(image: SandboxImage) {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (event.event !== 'images://build' || event.payload.image !== image)
        return
      const { line, finished, error } = event.payload
      const known = queryClient.getQueryData<ImageStatus>(
        imageQueryKeys.status(image)
      )
      // A job the core started (a run that needed the image): ask what it is
      if (finished === null && known && !known.building && !known.pulling)
        void queryClient.invalidateQueries({
          queryKey: imageQueryKeys.status(image),
        })
      queryClient.setQueryData<ImageStatus>(
        imageQueryKeys.status(image),
        current =>
          current && {
            ...current,
            // A line while busy keeps the job going; the end clears both
            building: finished === null && current.building,
            pulling: finished === null && current.pulling,
            log: line ? [...current.log, line].slice(-LOG_LINES) : current.log,
            error: finished === false ? error : current.error,
          }
      )
      if (finished !== null) {
        void queryClient.invalidateQueries({
          queryKey: imageQueryKeys.status(image),
        })
        // The runtime cards say whether the sandbox is built
        void queryClient.invalidateQueries({ queryKey: harnessQueryKeys.all })
      }
    })
    return () => subscription.unsubscribe()
  }, [image, queryClient])

  return useQuery({
    queryKey: imageQueryKeys.status(image),
    queryFn: async () => {
      const result = await client.getImageStatus(image)
      if (result.status === 'error') throw new Error(result.error)
      return result.data
    },
  })
}

export function useBuildImage(image: SandboxImage) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async () => {
      const result = await client.buildImage(image)
      if (result.status === 'error') throw new Error(result.error)
    },
    onMutate: () =>
      queryClient.setQueryData<ImageStatus>(
        imageQueryKeys.status(image),
        current =>
          current && { ...current, building: true, log: [], error: null }
      ),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: imageQueryKeys.status(image) }),
  })
}

/** "Download": pulls the image pinned to this app from the registry */
export function usePullImage(image: SandboxImage) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async () => {
      const result = await client.pullImage(image)
      if (result.status === 'error') throw new Error(result.error)
    },
    onMutate: () =>
      queryClient.setQueryData<ImageStatus>(
        imageQueryKeys.status(image),
        current =>
          current && { ...current, pulling: true, log: [], error: null }
      ),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: imageQueryKeys.status(image) }),
  })
}
