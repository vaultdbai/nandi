import { useEffect, useState } from 'react'
import { keepPreviousData, useQuery } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import { toast } from 'sonner'
import {
  askItem,
  fromCommand,
  fromServer,
  groupItems,
  parseQuery,
  scoreText,
  type PaletteGroup,
  type PaletteItem,
} from '@/components/palette/palette-items'
import { addRecent, loadRecents } from '@/components/palette/recents'
import { openNewChat, sendToStudio } from '@/lib/agent-actions'
import { client } from '@/lib/client'
import { executeCommand, getAllCommands } from '@/lib/commands/registry'
import { useCommandContext } from '@/hooks/use-command-context'
import { unwrapResult } from '@/lib/tauri-bindings'
import { parseView, useNavigationStore } from '@/store/navigation-store'

/** Wait this long after the last keystroke before searching */
export const SEARCH_DEBOUNCE_MS = 150

/** Recent empty-query suggestions stay fresh this long */
const STALE_MS = 10_000

function useDebounced<T>(value: T, ms: number): T {
  const [debounced, setDebounced] = useState(value)
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), ms)
    return () => clearTimeout(timer)
  }, [value, ms])
  return debounced
}

export const paletteQueryKeys = {
  search: (query: string) => ['palette', 'search', query] as const,
}

/**
 * Palette search and execution. Server results (`searchPalette`, debounced)
 * are merged with the local command registry (sidebars, preferences,
 * window); an empty query shows recent items and suggestions.
 */
export function usePalette(open: boolean) {
  const { t } = useTranslation()
  const commandContext = useCommandContext()
  const [query, setQuery] = useState('')
  const debounced = useDebounced(query, SEARCH_DEBOUNCE_MS)
  const { commandMode, text } = parseQuery(query)
  const serverQuery = debounced.trim()

  const search = useQuery({
    queryKey: paletteQueryKeys.search(serverQuery),
    queryFn: async () => unwrapResult(await client.searchPalette(serverQuery)),
    enabled: open,
    staleTime: STALE_MS,
    placeholderData: keepPreviousData,
  })

  const local = getAllCommands(commandContext)
    .map(command => fromCommand(command, t))
    .map(item => ({
      ...item,
      score: scoreText(text, item.name, item.description),
    }))
    // Without a query, local commands are only listed in command mode
    .filter(item => (text ? item.score > 0 : commandMode))

  const server = (search.data ?? [])
    .map(fromServer)
    .filter(item => !commandMode || item.kind === 'action')

  const groups: PaletteGroup[] = []
  if (!text && !commandMode) {
    const recents = loadRecents()
    if (recents.length > 0) groups.push({ id: 'recent', items: recents })
    const seen = new Set(recents.map(item => item.key))
    groups.push(...groupItems(server.filter(item => !seen.has(item.key))))
  } else {
    // Typed text can always go to Studio: the first row
    if (text && !commandMode) {
      groups.push({ id: 'ask', items: [askItem(text, t)] })
    }
    groups.push(...groupItems([...local, ...server]))
  }
  const items = groups.flatMap(group => group.items)

  const execute = async (item: PaletteItem) => {
    if (item.kind === 'ask') {
      await sendToStudio(String(item.args ?? text), [], null)
      return
    }
    addRecent({ ...item, score: 0 })
    if (item.source === 'local') {
      const result = await executeCommand(item.id, commandContext)
      if (!result.success && result.error) toast.error(result.error)
      return
    }
    if (item.kind === 'action') {
      // An empty chat; its first message creates the conversation
      if (item.id === 'new_studio_chat') {
        openNewChat()
        return
      }
      const result = await client.executePaletteAction(
        item.id,
        item.args ?? null
      )
      if (result.status === 'error') {
        toast.error(result.error)
        return
      }
      if (result.data.message) toast.success(result.data.message)
      const view = parseView(result.data.navigate)
      if (view) useNavigationStore.getState().navigate(view)
      return
    }
    const view = parseView(item.navigate)
    if (view) useNavigationStore.getState().navigate(view)
  }

  return {
    query,
    setQuery,
    commandMode,
    groups,
    items,
    isLoading: search.isFetching,
    error: search.error,
    execute,
  }
}
