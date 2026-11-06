/**
 * Live UI: what a file change on disk means for the open screens.
 *
 * The core sends `live://changed` for every change to a file the app
 * shows, whoever made it (`services::live`). Each change invalidates
 * exactly the queries that show it (`queryKeysFor`) and goes to the top of
 * the recent-changes list. The Refresh button and F5 / Ctrl+R
 * (`refreshFromDisk`) re-read everything from disk, for when a watcher
 * missed something.
 */

import type { QueryKey } from '@tanstack/react-query'
import { agentPoliciesQueryKeys } from '@/hooks/useAgentPolicies'
import { gatewayQueryKeys } from '@/hooks/useGateway'
import { marketplaceQueryKeys } from '@/hooks/useMarketplace'
import { notesKeys } from '@/hooks/useNotes'
import { policiesQueryKeys } from '@/hooks/usePolicies'
import { client } from '@/lib/client'
import type { LiveChange, RefreshReport } from '@/lib/bindings'
import { queryClient } from '@/lib/query-client'
import { preferencesQueryKeys } from '@/services/preferences'

export const liveQueryKeys = {
  recent: ['liveChanges', 'recent'] as const,
}

/** How many changes the recent list holds (the core keeps 200) */
const RECENT_LIMIT = 200

/** The queries that show what `change` changed, and nothing else */
export function queryKeysFor(change: LiveChange): QueryKey[] {
  const id = change.targetId
  switch (change.area) {
    case 'agent':
      // The list follows `agent://agents_changed`
      return id
        ? [['agent-definition', id], agentPoliciesQueryKeys.agent(id)]
        : [['agent-definition'], agentPoliciesQueryKeys.all]
    case 'workflow':
      // The list follows `workflow://changed`
      return [id ? ['workflow', id] : ['workflow']]
    case 'trigger':
      // The list follows `trigger://changed`
      return [id ? ['trigger', id] : ['trigger']]
    case 'policy':
      // A policy may cover any agent
      return [agentPoliciesQueryKeys.all, policiesQueryKeys.all]
    case 'settings':
      return [preferencesQueryKeys.all]
    case 'connectors':
      return [['connectors']]
    case 'packs':
      return [marketplaceQueryKeys.all]
    case 'profile':
      return [
        gatewayQueryKeys.manifest,
        gatewayQueryKeys.config,
        gatewayQueryKeys.status,
      ]
    case 'notes':
      return id ? [notesKeys.list, notesKeys.note(id)] : [notesKeys.all]
    // Shown only in the recent list (no screen lists a work folder)
    case 'work':
    case 'manifest':
      return []
  }
}

/** Every query backed by files: what Refresh brings up to date */
export const FILE_BACKED_KEYS: QueryKey[] = [
  ['agents'],
  ['agent-definition'],
  ['agentMemory'],
  agentPoliciesQueryKeys.all,
  policiesQueryKeys.all,
  ['workflows'],
  ['workflow'],
  ['triggers'],
  ['trigger'],
  ['capabilities'],
  ['connectors'],
  marketplaceQueryKeys.all,
  preferencesQueryKeys.all,
  gatewayQueryKeys.manifest,
  gatewayQueryKeys.config,
  notesKeys.all,
  ['git'],
  ['git-status'],
  liveQueryKeys.recent,
]

/** Applies one change: the affected queries refetch, the list grows */
export function applyLiveChange(change: LiveChange) {
  for (const queryKey of queryKeysFor(change)) {
    void queryClient.invalidateQueries({ queryKey })
  }
  queryClient.setQueryData<LiveChange[]>(liveQueryKeys.recent, recent =>
    recent === undefined
      ? undefined
      : [change, ...recent.filter(c => c.id !== change.id)].slice(
          0,
          RECENT_LIMIT
        )
  )
}

/**
 * Re-reads every definition from disk (watcher or not), then refetches
 * every file-backed query. Notes and work folders are read from disk on
 * every request, so refetching them is enough.
 */
export async function refreshFromDisk(): Promise<RefreshReport> {
  const result = await client.refreshFromDisk()
  if (result.status === 'error') throw new Error(result.error)
  const report = result.data
  await Promise.all(
    FILE_BACKED_KEYS.map(queryKey =>
      queryClient.invalidateQueries({ queryKey })
    )
  )
  return report
}
