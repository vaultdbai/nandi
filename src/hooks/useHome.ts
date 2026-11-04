/**
 * Home's data, shared with the companion (task 16): runs and delegations as
 * activity rows, and Running / Recent / tiles built from them and the work
 * items. Live through the sources' own events (runs: `agent://*`,
 * delegations: `task://*`, work: `work://*`).
 */
import { useState } from 'react'
import { useNow } from '@/components/agents/task-utils'
import {
  activityRows,
  type ActivityRow,
} from '@/components/home/activity-model'
import {
  buildHomeFeed,
  homeTiles,
  RECENT_WINDOW_MS,
} from '@/components/home/home-model'
import { useAgents } from '@/hooks/useAgents'
import { useDelegation } from '@/hooks/useDelegation'
import { useInbox } from '@/hooks/useInbox'
import { useRuns } from '@/hooks/useRuns'
import { useWork } from '@/hooks/useWork'

/** Runs Home reads: one server page (Recent shows 8) */
const HOME_RUNS = 200

/** A name for every agent ID (the ID while agents load) */
export function useAgentNames(): (id: string) => string {
  const { data: agents = [] } = useAgents()
  const names = new Map(agents.map(agent => [agent.id, agent.name]))
  return id => names.get(id) ?? id
}

/**
 * Runs from the last day plus every delegation, as activity rows. The
 * window is fixed when Home opens, so the query key stays stable.
 */
export function useRecentActivity(): {
  rows: ActivityRow[]
  isPending: boolean
} {
  const [since] = useState(() => Date.now() - RECENT_WINDOW_MS)
  const runs = useRuns({ dateFrom: since }, 0, HOME_RUNS)
  const delegation = useDelegation()
  const agentName = useAgentNames()
  return {
    rows: activityRows(runs.data?.rows ?? [], delegation.history, agentName),
    isPending: runs.isPending,
  }
}

export function useHomeFeed() {
  // Elapsed times and the 24 h window move without an event
  const now = useNow(true, 30_000)
  const work = useWork(null)
  const activity = useRecentActivity()
  const inbox = useInbox()
  const feed = buildHomeFeed({
    work: work.data ?? [],
    activity: activity.rows,
    now,
  })
  return {
    ...feed,
    tiles: homeTiles({
      activity: activity.rows,
      work: work.data ?? [],
      waiting: inbox.count,
      now,
    }),
    inbox,
    now,
    isPending: work.isPending || activity.isPending,
  }
}
