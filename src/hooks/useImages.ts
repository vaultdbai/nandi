/**
 * A sandbox image: whether it's here, its download size, and a download's
 * progress, kept live by `images://pull` events; and the tool bundles
 * fetched on first use (Claude Code), kept live by `images://tools`. Both
 * run in the core, so leaving the page doesn't stop them; coming back
 * shows the log so far.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type {
  ImageStatus,
  SandboxImage,
  ToolBundle,
  ToolsStatus,
} from '@/lib/bindings'
import { harnessQueryKeys } from '@/hooks/useHarnesses'

export const imageQueryKeys = {
  status: (image: SandboxImage) => ['images', image] as const,
  tools: (bundle: ToolBundle) => ['images', 'tools', bundle] as const,
}

/** Lines of output shown */
const LOG_LINES = 300

export function useImageStatus(image: SandboxImage) {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (event.event !== 'images://pull' || event.payload.image !== image)
        return
      const { line, progress, finished, error } = event.payload
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
            progress: progress ?? current.progress,
            log: line ? [...current.log, line].slice(-LOG_LINES) : current.log,
            error: finished === false ? error : current.error,
          }
      )
      if (finished !== null) {
        void queryClient.invalidateQueries({
          queryKey: imageQueryKeys.status(image),
        })
        // The runtime cards say whether the sandbox is here
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

/** The advanced option: builds the image here */
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
          current && {
            ...current,
            pulling: true,
            progress: null,
            log: [],
            error: null,
          }
      ),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: imageQueryKeys.status(image) }),
  })
}

export function useToolsStatus(bundle: ToolBundle) {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (event.event !== 'images://tools' || event.payload.bundle !== bundle)
        return
      const { line, finished, error } = event.payload
      queryClient.setQueryData<ToolsStatus>(
        imageQueryKeys.tools(bundle),
        current =>
          current && {
            ...current,
            installing: finished === null,
            ready: finished === true || current.ready,
            log: line ? [...current.log, line].slice(-LOG_LINES) : current.log,
            error: finished === false ? error : current.error,
          }
      )
      if (finished !== null)
        void queryClient.invalidateQueries({
          queryKey: imageQueryKeys.tools(bundle),
        })
    })
    return () => subscription.unsubscribe()
  }, [bundle, queryClient])

  return useQuery({
    queryKey: imageQueryKeys.tools(bundle),
    queryFn: async () => {
      const result = await client.getToolsStatus(bundle)
      if (result.status === 'error') throw new Error(result.error)
      return result.data
    },
  })
}

/** "Get it now" / Retry for a first-use tool bundle */
export function useInstallTools(bundle: ToolBundle) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async () => {
      const result = await client.installTools(bundle)
      if (result.status === 'error') throw new Error(result.error)
    },
    onMutate: () =>
      queryClient.setQueryData<ToolsStatus>(
        imageQueryKeys.tools(bundle),
        current =>
          current && { ...current, installing: true, log: [], error: null }
      ),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: imageQueryKeys.tools(bundle) }),
  })
}
