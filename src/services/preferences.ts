import {
  useQuery,
  useMutation,
  useQueryClient,
  type QueryClient,
} from '@tanstack/react-query'
import { toast } from 'sonner'
import { logger } from '@/lib/logger'
import { client } from '@/lib/client'
import type { AppPreferences } from '@/lib/tauri-bindings'

// Query keys for preferences
export const preferencesQueryKeys = {
  all: ['preferences'] as const,
  preferences: () => [...preferencesQueryKeys.all] as const,
}

// TanStack Query hooks following the architectural patterns
export function usePreferences() {
  return useQuery({
    queryKey: preferencesQueryKeys.preferences(),
    queryFn: async (): Promise<AppPreferences> => {
      logger.debug('Loading preferences from backend')
      const result = await client.loadPreferences()

      if (result.status === 'error') {
        // Return defaults if preferences file doesn't exist yet
        logger.warn('Failed to load preferences, using defaults', {
          error: result.error,
        })
        return { theme: 'system', language: null }
      }

      logger.info('Preferences loaded successfully', {
        preferences: result.data,
      })
      return result.data
    },
    staleTime: 1000 * 60 * 5, // 5 minutes
    gcTime: 1000 * 60 * 10, // 10 minutes
  })
}

/**
 * Saves which panes show, quietly (no toast: a click on a pane toggle is not
 * a settings change). On failure the panes still work; the next launch just
 * starts from the saved layout.
 */
export async function savePaneLayout(
  queryClient: QueryClient,
  layout: Pick<AppPreferences, 'sidebarVisible' | 'workTrayVisible'>
) {
  const current = queryClient.getQueryData<AppPreferences>(
    preferencesQueryKeys.preferences()
  )
  if (!current) return
  const next = { ...current, ...layout }
  const result = await client.savePreferences(next)
  if (result.status === 'error') {
    logger.warn('Failed to save the pane layout', { error: result.error })
    return
  }
  queryClient.setQueryData(preferencesQueryKeys.preferences(), next)
}

export function useSavePreferences() {
  const queryClient = useQueryClient()

  return useMutation({
    mutationFn: async (preferences: AppPreferences) => {
      logger.debug('Saving preferences to backend', { preferences })
      const result = await client.savePreferences(preferences)

      if (result.status === 'error') {
        logger.error('Failed to save preferences', {
          error: result.error,
          preferences,
        })
        toast.error('Failed to save preferences', { description: result.error })
        throw new Error(result.error)
      }

      logger.info('Preferences saved successfully')
    },
    onSuccess: (_, preferences) => {
      // Update the cache with the new preferences
      queryClient.setQueryData(preferencesQueryKeys.preferences(), preferences)
      logger.info('Preferences cache updated')
      toast.success('Preferences saved')
    },
  })
}
