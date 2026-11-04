import { describe, expect, it } from 'vitest'
import {
  agentCard,
  sharedAgent,
  task,
  team,
  trustRule,
} from '@/hooks/useSharedAgents.fixtures'
import {
  CALL_WINDOW_MS,
  buildTrustGraph,
  edgeWidth,
  layoutTrustGraph,
  neighborhood,
  policyFor,
  teamNodeId,
  type TrustGraphInput,
  type TrustGraphModel,
} from './trust-graph-model'

const NOW = Date.UTC(2026, 9, 9)
const DAY = 24 * 60 * 60 * 1000

const mine = sharedAgent(agentCard('me', 'papers', { name: 'Papers' }), {
  sources: ['team', 'relay'],
  teamIds: ['acme'],
})
const remote = sharedAgent(agentCard('bob', 'atlas', { name: 'Atlas' }), {
  sources: ['relay'],
})

function graph(overrides: Partial<TrustGraphInput> = {}) {
  return buildTrustGraph({
    sharedByMe: [mine],
    sharedWithMe: [remote],
    history: [],
    teams: [team('acme')],
    policies: [],
    now: NOW,
    ...overrides,
  })
}

const kindOf = (model: TrustGraphModel, id: string) =>
  model.nodes.find(n => n.id === id)?.kind
const edgeOf = (model: TrustGraphModel, id: string) =>
  model.edges.find(e => e.id === id)

describe('buildTrustGraph', () => {
  it('has my agents, agents I can call, and teams linked to their agents', () => {
    const model = graph()
    expect(kindOf(model, 'me/papers')).toBe('mine')
    expect(kindOf(model, 'bob/atlas')).toBe('callable')
    expect(kindOf(model, teamNodeId('acme'))).toBe('team')
    expect(model.edges).toEqual([
      expect.objectContaining({
        kind: 'team',
        source: 'team:acme',
        target: 'me/papers',
        policy: null,
      }),
    ])
  })

  it('weights call edges by calls in the last 30 days, in both directions', () => {
    const model = graph({
      history: [
        // Bare agent IDs resolve to my card
        task('inbound', 'carol/scribe', 'papers', NOW - DAY),
        task('inbound', 'carol/scribe', 'me/papers', NOW - 2 * DAY),
        task('inbound', 'carol/scribe', 'papers', NOW - CALL_WINDOW_MS - DAY),
        task('outbound', 'papers', 'bob/atlas', NOW - DAY),
        task('outbound', 'me/papers', 'dave/new', NOW - DAY),
      ],
    })
    expect(kindOf(model, 'carol/scribe')).toBe('caller')
    expect(edgeOf(model, 'carol/scribe->me/papers')).toMatchObject({
      kind: 'call',
      weight: 2,
      policy: 'ask',
    })
    expect(edgeOf(model, 'me/papers->bob/atlas')).toMatchObject({
      weight: 1,
      policy: null,
    })
    // Seen only in history: still something I can call
    expect(kindOf(model, 'dave/new')).toBe('callable')
    expect(model.nodes.filter(n => n.id === 'me/papers')).toHaveLength(1)
    expect(model.nodes.find(n => n.id === 'me/papers')?.calls).toBe(4)
  })

  it('carries my policy on inbound edges and keeps ruled callers visible', () => {
    const policies = [
      trustRule('*', 'papers', 'deny'),
      trustRule('carol/scribe', 'papers', 'allow'),
      trustRule('eve/spy', 'papers', 'deny'),
    ]
    const model = graph({
      policies,
      history: [task('inbound', 'carol/scribe', 'papers', NOW - DAY)],
    })
    expect(edgeOf(model, 'carol/scribe->me/papers')?.policy).toBe('allow')
    expect(edgeOf(model, 'eve/spy->me/papers')).toMatchObject({
      weight: 0,
      policy: 'deny',
    })
    expect(kindOf(model, 'eve/spy')).toBe('caller')
    expect(policyFor(policies, 'frank/x', 'me/papers')).toBe('deny')
    expect(policyFor([], 'frank/x', 'me/papers')).toBe('ask')
  })

  it('treats an agent that both calls me and that I can call as callable', () => {
    const model = graph({
      history: [task('inbound', 'bob/atlas', 'papers', NOW - DAY)],
    })
    expect(kindOf(model, 'bob/atlas')).toBe('callable')
  })

  it('finds neighborhoods and scales edges', () => {
    const model = graph({
      history: [task('inbound', 'carol/scribe', 'papers', NOW - DAY)],
    })
    expect([...neighborhood(model, 'me/papers')].sort()).toEqual([
      'carol/scribe',
      'me/papers',
      'team:acme',
    ])
    expect(edgeWidth(0)).toBe(1)
    expect(edgeWidth(8)).toBeGreaterThan(edgeWidth(1))
    expect(edgeWidth(10_000)).toBe(6)
  })

  it('lays every node out inside the canvas', () => {
    const model = graph({
      history: [task('inbound', 'carol/scribe', 'papers', NOW - DAY)],
    })
    const positions = layoutTrustGraph(model, 800, 520)
    expect(positions.size).toBe(model.nodes.length)
    for (const { x, y } of positions.values()) {
      expect(x).toBeGreaterThan(0)
      expect(x).toBeLessThan(800)
      expect(y).toBeGreaterThan(0)
      expect(y).toBeLessThan(520)
    }
  })
})
