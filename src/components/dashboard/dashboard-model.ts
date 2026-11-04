/**
 * Pure rules for the Dashboard's first-run hints: when the page is still
 * empty, which Get started cards apply, and when to nudge about backup.
 */
import type { DashboardSnapshot, PackManifest } from '@/lib/bindings'

export const GET_STARTED_KEY = 'getStarted'
export const BACKUP_NUDGE_KEY = 'backupNudge'
/** "Not now" on the backup nudge */
export const BACKUP_SNOOZE_MS = 30 * 24 * 60 * 60 * 1000
/** Suggestions shown in the welcome strip */
const MAX_SUGGESTIONS = 2

/** Whether there's an agent besides the built-in ones (Chief of Staff) */
const hasOwnAgents = (snapshot: DashboardSnapshot) =>
  snapshot.agentHealth.some(health => !health.agent.builtin)

/**
 * No agents but the built-in ones, and nothing has happened: show Get
 * started, not zero tiles
 */
export function isFreshDashboard(snapshot: DashboardSnapshot): boolean {
  return (
    !hasOwnAgents(snapshot) &&
    snapshot.recentActivity.length === 0 &&
    snapshot.recentOutputs.length === 0 &&
    snapshot.recentTriggerFires.length === 0 &&
    snapshot.attentionItems.length === 0
  )
}

export type GetStartedCard = 'packs' | 'studio' | 'network'

/**
 * The cards that still apply: packs until there's an agent of the user's
 * own (installed or made), the network until a relay is connected. Studio
 * stays while the section shows. The section goes once anything has
 * finished, or once it's dismissed.
 */
export function getStartedCards(input: {
  snapshot: DashboardSnapshot
  relayConnected: boolean
  dismissed: boolean
}): GetStartedCard[] {
  const { snapshot, relayConnected, dismissed } = input
  if (dismissed || snapshot.recentOutputs.length > 0) return []
  const packs = !hasOwnAgents(snapshot)
  const network = !relayConnected
  // Studio alone isn't worth a section
  if (!packs && !network) return []
  const cards: GetStartedCard[] = []
  if (packs) cards.push('packs')
  cards.push('studio')
  if (network) cards.push('network')
  return cards
}

/**
 * Nudge once work has finished, as long as there's no backup remote and
 * the user hasn't said "Not now" lately
 */
export function showBackupNudge(input: {
  snapshot: DashboardSnapshot
  remoteConfigured: boolean | undefined
  dismissed: boolean
}): boolean {
  return (
    input.remoteConfigured === false &&
    !input.dismissed &&
    input.snapshot.recentOutputs.length > 0
  )
}

export interface Welcome {
  packNames: string[]
  /** The agents the packs just installed */
  agentNames: string[]
  /** Every agent the user now has: the built-in ones first, then those */
  allAgentNames: string[]
  suggestions: string[]
}

/**
 * What the welcome strip says about the packs just installed, and the
 * built-in agents (Chief of Staff) the user has whatever they installed
 */
export function welcomeFor(
  packIds: string[],
  packs: PackManifest[] | undefined,
  builtinNames: string[] = []
): Welcome {
  const installed = (packs ?? []).filter(pack => packIds.includes(pack.id))
  const agentNames = [
    ...new Set(installed.flatMap(pack => pack.agents.map(a => a.name))),
  ]
  return {
    packNames: installed.map(pack => pack.name),
    agentNames,
    allAgentNames: [...new Set([...builtinNames, ...agentNames])],
    suggestions: installed
      .flatMap(pack => pack.suggestedAsks)
      .slice(0, MAX_SUGGESTIONS),
  }
}
