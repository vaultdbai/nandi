/**
 * The trust graph's nodes and edges, built from what's shared and the
 * delegation history, and their force-directed layout. Pure (no React, no
 * I/O) so it's unit-tested apart from the rendering in TrustGraph.tsx.
 */

import {
  forceCenter,
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  forceX,
  forceY,
  type SimulationLinkDatum,
  type SimulationNodeDatum,
} from 'd3-force'
import type {
  SharedAgent,
  TaskResult,
  Team,
  TrustPolicy,
  TrustRule,
} from '@/lib/bindings'

/** Call counts cover the last 30 days, like the backend's `call_count` */
export const CALL_WINDOW_MS = 30 * 24 * 60 * 60 * 1000

/**
 * - `mine`: my agents (green)
 * - `callable`: agents I can call (blue); wins over `caller` when both
 * - `caller`: agents that call mine (amber)
 * - `team`: a team (grey)
 */
export type TrustNodeKind = 'mine' | 'callable' | 'caller' | 'team'

export interface TrustNode {
  /** A card ID, or `team:<teamId>` */
  id: string
  kind: TrustNodeKind
  label: string
  /** Owner handle or team org */
  detail: string
  /** Delegations in the window, either direction */
  calls: number
}

/** `call`: `source` (requester) delegated to `target` (provider) */
export type TrustEdgeKind = 'call' | 'team'

export interface TrustEdge {
  id: string
  kind: TrustEdgeKind
  source: string
  target: string
  /** Calls in the window (0 for team edges and rule-only edges) */
  weight: number
  /**
   * Inbound call edges (someone calling one of my agents) carry my policy
   * for that caller; `null` on edges I can't set a policy for
   */
  policy: TrustPolicy | null
}

export interface TrustGraphModel {
  nodes: TrustNode[]
  edges: TrustEdge[]
}

export interface TrustGraphInput {
  sharedByMe: SharedAgent[]
  sharedWithMe: SharedAgent[]
  history: TaskResult[]
  teams: Team[]
  policies: TrustRule[]
  now: number
}

export const teamNodeId = (teamId: string) => `team:${teamId}`

/** The agent ID part of a card ID (`<instance>/<agent>`) */
export function agentIdOf(cardId: string): string {
  const slash = cardId.lastIndexOf('/')
  return slash === -1 ? cardId : cardId.slice(slash + 1)
}

/** My policy for `requester` calling `providerCardId`: its rule, else `*`, else ask */
export function policyFor(
  policies: TrustRule[],
  requester: string,
  providerCardId: string
): TrustPolicy {
  const agentId = agentIdOf(providerCardId)
  const forAgent = policies.filter(
    rule =>
      rule.providerAgent === agentId || rule.providerAgent === providerCardId
  )
  return (
    forAgent.find(rule => rule.requester === requester)?.policy ??
    forAgent.find(rule => rule.requester === '*')?.policy ??
    'ask'
  )
}

export function buildTrustGraph(input: TrustGraphInput): TrustGraphModel {
  const { sharedByMe, sharedWithMe, history, teams, policies, now } = input
  const nodes = new Map<string, TrustNode>()
  const edges = new Map<string, TrustEdge>()

  // My agents: by card ID, also reachable by bare agent ID (history rows
  // may use either)
  const mineByAgentId = new Map<string, string>()
  for (const shared of sharedByMe) {
    const { card } = shared
    nodes.set(card.id, {
      id: card.id,
      kind: 'mine',
      label: card.name,
      detail: card.owner.githubHandle,
      calls: 0,
    })
    mineByAgentId.set(agentIdOf(card.id), card.id)
  }
  const resolveMine = (id: string): string | null => {
    if (nodes.get(id)?.kind === 'mine') return id
    return mineByAgentId.get(id) ?? null
  }

  const ensureRemote = (
    id: string,
    kind: 'callable' | 'caller',
    label?: string,
    detail?: string
  ) => {
    const existing = nodes.get(id)
    if (existing) {
      if (existing.kind === 'caller' && kind === 'callable') {
        existing.kind = 'callable'
      }
      return existing
    }
    const node: TrustNode = {
      id,
      kind,
      label: label ?? agentIdOf(id),
      detail: detail ?? '',
      calls: 0,
    }
    nodes.set(id, node)
    return node
  }

  for (const shared of sharedWithMe) {
    const { card } = shared
    if (resolveMine(card.id)) continue
    ensureRemote(card.id, 'callable', card.name, card.owner.githubHandle)
  }

  // Teams, linked to the agents shared through them
  const teamNames = new Map(teams.map(team => [team.id, team]))
  const ensureTeam = (teamId: string) => {
    const nodeId = teamNodeId(teamId)
    if (!nodes.has(nodeId)) {
      const team = teamNames.get(teamId)
      nodes.set(nodeId, {
        id: nodeId,
        kind: 'team',
        label: team?.name ?? teamId,
        detail: team?.githubOrg ?? teamId,
        calls: 0,
      })
    }
    return nodeId
  }
  const linkTeam = (teamId: string, cardId: string) => {
    const nodeId = ensureTeam(teamId)
    const id = `${nodeId}--${cardId}`
    edges.set(id, {
      id,
      kind: 'team',
      source: nodeId,
      target: cardId,
      weight: 0,
      policy: null,
    })
  }
  // A team with no agents still shows
  for (const team of teams) ensureTeam(team.id)
  for (const shared of [...sharedByMe, ...sharedWithMe]) {
    const cardId = resolveMine(shared.card.id) ?? shared.card.id
    for (const teamId of shared.teamIds) linkTeam(teamId, cardId)
  }

  // Delegations in the window
  const since = now - CALL_WINDOW_MS
  const callEdge = (source: string, target: string, inbound: boolean) => {
    const id = `${source}->${target}`
    let edge = edges.get(id)
    if (!edge) {
      edge = {
        id,
        kind: 'call',
        source,
        target,
        weight: 0,
        policy: inbound ? policyFor(policies, source, target) : null,
      }
      edges.set(id, edge)
    }
    return edge
  }
  for (const task of history) {
    if (task.startedAt < since) continue
    let source: string
    let target: string
    if (task.direction === 'inbound') {
      target = resolveMine(task.providerAgent) ?? task.providerAgent
      source = task.requesterAgent
      if (!nodes.has(target)) {
        nodes.set(target, {
          id: target,
          kind: 'mine',
          label: agentIdOf(target),
          detail: task.providerOwner ?? '',
          calls: 0,
        })
      }
      ensureRemote(source, 'caller', undefined, task.requesterOwner ?? '')
    } else {
      source = resolveMine(task.requesterAgent) ?? task.requesterAgent
      target = task.providerAgent
      if (!nodes.has(source)) {
        nodes.set(source, {
          id: source,
          kind: 'mine',
          label: agentIdOf(source),
          detail: task.requesterOwner ?? '',
          calls: 0,
        })
      }
      ensureRemote(target, 'callable', undefined, task.providerOwner ?? '')
    }
    const edge = callEdge(source, target, task.direction === 'inbound')
    edge.weight += 1
    const sourceNode = nodes.get(source)
    const targetNode = nodes.get(target)
    if (sourceNode) sourceNode.calls += 1
    if (targetNode) targetNode.calls += 1
  }

  // Callers with a rule but no recent calls stay visible (a deny, say)
  for (const rule of policies) {
    if (rule.requester === '*') continue
    const target = resolveMine(rule.providerAgent)
    if (!target) continue
    ensureRemote(rule.requester, 'caller')
    callEdge(rule.requester, target, true)
  }

  return { nodes: [...nodes.values()], edges: [...edges.values()] }
}

/** IDs of `nodeId` and every node one edge away */
export function neighborhood(
  model: TrustGraphModel,
  nodeId: string
): Set<string> {
  const ids = new Set([nodeId])
  for (const edge of model.edges) {
    if (edge.source === nodeId) ids.add(edge.target)
    if (edge.target === nodeId) ids.add(edge.source)
  }
  return ids
}

/** Stroke width for an edge: 1px, growing with the log of its calls */
export function edgeWidth(weight: number): number {
  return weight <= 0 ? 1 : 1 + Math.min(5, Math.log2(weight + 1) * 1.5)
}

export interface NodePosition {
  x: number
  y: number
}

/** Ticks the force simulation runs before drawing (no animation) */
const LAYOUT_TICKS = 300

/**
 * Force-directed positions for `model` inside `width`×`height`. Runs the
 * simulation to rest synchronously; d3-force's seeded random source makes
 * it deterministic for the same input.
 */
export function layoutTrustGraph(
  model: TrustGraphModel,
  width: number,
  height: number
): Map<string, NodePosition> {
  type SimNode = SimulationNodeDatum & { id: string }
  const simNodes: SimNode[] = model.nodes.map(node => ({ id: node.id }))
  const ids = new Set(simNodes.map(node => node.id))
  const simLinks: SimulationLinkDatum<SimNode>[] = model.edges
    .filter(edge => ids.has(edge.source) && ids.has(edge.target))
    .map(edge => ({ source: edge.source, target: edge.target }))
  const simulation = forceSimulation(simNodes)
    .force(
      'link',
      forceLink<SimNode, SimulationLinkDatum<SimNode>>(simLinks)
        .id(node => node.id)
        .distance(90)
    )
    .force('charge', forceManyBody().strength(-260))
    .force('center', forceCenter(width / 2, height / 2))
    .force('collide', forceCollide(32))
    .force('x', forceX(width / 2).strength(0.05))
    .force('y', forceY(height / 2).strength(0.05))
    .stop()
  simulation.tick(LAYOUT_TICKS)

  const margin = 36
  const clamp = (value: number, max: number) =>
    Math.min(max - margin, Math.max(margin, value))
  return new Map(
    simNodes.map(node => [
      node.id,
      {
        x: clamp(node.x ?? width / 2, width),
        y: clamp(node.y ?? height / 2, height),
      },
    ])
  )
}
