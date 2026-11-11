/**
 * The unified tool catalog (TanStack Query): built-in, connector, and
 * pilot tools. Refetched on `tool://catalog_changed` (the core rebuilds it
 * when connectors or pilots are installed, started, or stopped).
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { client } from '@/lib/client'
import type { ToolEntry, ToolUsageStats } from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

/** Everything tool-related lives under `['tools']` */
export const toolQueryKeys = {
  all: ['tools'] as const,
  list: ['tools', 'list'] as const,
  tool: (toolId: string) => ['tools', 'tool', toolId] as const,
  usage: ['tools', 'usage'] as const,
}

/** Refetches the catalog when the core says it changed */
function useToolEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (
        event.event === 'tool://catalog_changed' ||
        event.event === 'tool://usage_recorded'
      ) {
        void queryClient.invalidateQueries({ queryKey: toolQueryKeys.all })
      }
    })
    return subscription.unsubscribe
  }, [queryClient])
}

/** Every tool in the catalog, sorted by ID, with 30-day usage */
export function useTools() {
  useToolEvents()
  return useQuery({
    queryKey: toolQueryKeys.list,
    queryFn: async (): Promise<ToolEntry[]> =>
      unwrapResult(await client.listTools()),
  })
}

/** One tool by ID, bare built-in name, or runtime name (idle while `null`) */
export function useTool(toolId: string | null) {
  return useQuery({
    queryKey: toolQueryKeys.tool(toolId ?? ''),
    queryFn: async (): Promise<ToolEntry> =>
      unwrapResult(await client.getTool(toolId ?? '')),
    enabled: !!toolId,
  })
}

/** Calls and failures per tool in the last 30 days */
export function useToolUsageStats() {
  return useQuery({
    queryKey: toolQueryKeys.usage,
    queryFn: async (): Promise<ToolUsageStats[]> =>
      unwrapResult(await client.getToolUsageStats()),
  })
}

/** Turns a tool on or off; it stays in the catalog either way */
export function useSetToolEnabled() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      toolId,
      enabled,
    }: {
      toolId: string
      enabled: boolean
    }): Promise<ToolEntry> =>
      unwrapResult(await client.setToolEnabled(toolId, enabled)),
    onSuccess: updated => {
      queryClient.setQueryData<ToolEntry[]>(toolQueryKeys.list, current =>
        current?.map(entry =>
          entry.definition.id === updated.definition.id ? updated : entry
        )
      )
    },
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: toolQueryKeys.all }),
  })
}
