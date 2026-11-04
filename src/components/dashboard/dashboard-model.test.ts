import { describe, expect, it } from 'vitest'
import type { AgentHealth } from '@/lib/bindings'
import {
  agentInfo,
  dashboardSnapshot,
  packManifest,
  run,
} from '@/test/fixtures'
import {
  getStartedCards,
  isFreshDashboard,
  showBackupNudge,
  welcomeFor,
} from './dashboard-model'

const health = (id: string): AgentHealth => ({
  agent: agentInfo(id, id),
  lastRunAt: null,
  successRateWeek: null,
  avgDurationMs: null,
})

const withAgent = dashboardSnapshot({ agentHealth: [health('writer')] })
const withOutput = dashboardSnapshot({
  agentHealth: [health('writer')],
  recentOutputs: [
    { run: run('r1', 'writer', { status: 'completed' }), agentName: 'Writer' },
  ],
})

describe('isFreshDashboard', () => {
  it('is fresh only with no agents and nothing that happened', () => {
    expect(isFreshDashboard(dashboardSnapshot())).toBe(true)
    expect(isFreshDashboard(withAgent)).toBe(false)
  })
})

describe('getStartedCards', () => {
  const cards = (
    snapshot = dashboardSnapshot(),
    relayConnected = false,
    dismissed = false
  ) => getStartedCards({ snapshot, relayConnected, dismissed })

  it('offers all three on a fresh install', () => {
    expect(cards()).toEqual(['packs', 'studio', 'network'])
  })

  it('drops each card once it is done', () => {
    expect(cards(withAgent)).toEqual(['studio', 'network'])
    expect(cards(dashboardSnapshot(), true)).toEqual(['packs', 'studio'])
  })

  it('goes away when everything is done, work finished, or dismissed', () => {
    expect(cards(withAgent, true)).toEqual([])
    expect(cards(withOutput)).toEqual([])
    expect(cards(dashboardSnapshot(), false, true)).toEqual([])
  })
})

describe('showBackupNudge', () => {
  it('needs finished work, no remote, and no recent Not now', () => {
    const nudge = (
      snapshot = withOutput,
      remoteConfigured = false,
      dismissed = false
    ) => showBackupNudge({ snapshot, remoteConfigured, dismissed })

    expect(nudge()).toBe(true)
    expect(nudge(withAgent)).toBe(false)
    expect(nudge(withOutput, true)).toBe(false)
    // Unknown remote status: say nothing yet
    expect(
      showBackupNudge({
        snapshot: withOutput,
        remoteConfigured: undefined,
        dismissed: false,
      })
    ).toBe(false)
    expect(nudge(withOutput, false, true)).toBe(false)
  })
})

describe('welcomeFor', () => {
  it('names the installed packs, their agents, and up to two suggestions', () => {
    const packs = [
      packManifest('a', 'Alpha', {
        suggestedAsks: ['First', 'Second', 'Third'],
      }),
      packManifest('b', 'Beta'),
      packManifest('c', 'Gamma'),
    ]
    expect(welcomeFor(['a', 'b'], packs)).toEqual({
      packNames: ['Alpha', 'Beta'],
      agentNames: ['Alpha Agent', 'Beta Agent'],
      allAgentNames: ['Alpha Agent', 'Beta Agent'],
      suggestions: ['First', 'Second'],
    })
    expect(welcomeFor(['a'], undefined).packNames).toEqual([])
  })

  it('lists the built-in agents first, without counting them as installed', () => {
    const welcome = welcomeFor(
      ['a'],
      [packManifest('a', 'Alpha')],
      ['Chief of Staff']
    )
    expect(welcome.agentNames).toEqual(['Alpha Agent'])
    expect(welcome.allAgentNames).toEqual(['Chief of Staff', 'Alpha Agent'])
  })
})
