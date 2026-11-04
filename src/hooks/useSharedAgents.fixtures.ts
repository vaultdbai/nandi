/** Builders for the shared views' tests (Rust-shaped data). */

import { vi } from 'vitest'
import type {
  AgentCard,
  ProviderOption,
  SharedAgent,
  TaskResult,
  Team,
  TrustRule,
} from '@/lib/bindings'

export function agentCard(
  instanceId: string,
  agentId: string,
  overrides: Partial<AgentCard> = {}
): AgentCard {
  return {
    id: `${instanceId}/${agentId}`,
    name: agentId,
    description: '',
    instanceId,
    endpoint: '',
    capabilities: [{ name: 'find-papers' }],
    acceptsDelegation: true,
    maxConcurrent: 1,
    currentLoad: 0,
    owner: { githubHandle: `${instanceId}-owner`, displayName: instanceId },
    signature: null,
    issuedAt: 0,
    ...overrides,
  }
}

export function sharedAgent(
  card: AgentCard,
  overrides: Partial<SharedAgent> = {}
): SharedAgent {
  return {
    card,
    sources: ['team'],
    teamIds: [],
    callCount: 0,
    lastCalledAt: null,
    ...overrides,
  }
}

export function providerOption(
  card: AgentCard,
  overrides: Partial<ProviderOption> = {}
): ProviderOption {
  return {
    card,
    mine: false,
    online: true,
    teammate: false,
    sources: [],
    teamIds: [],
    stats: { calls: 0, succeeded: 0, failed: 0, avgDurationMs: null },
    successRate: null,
    rank: null,
    unavailable: null,
    ...overrides,
  }
}

export function task(
  direction: TaskResult['direction'],
  requesterAgent: string,
  providerAgent: string,
  startedAt: number
): TaskResult {
  return {
    taskId: `${requesterAgent}->${providerAgent}@${startedAt}`,
    providerAgent,
    output: null,
    status: 'completed',
    durationMs: 10,
    tokensIn: 0,
    tokensOut: 0,
    error: null,
    direction,
    capability: 'find-papers',
    requesterAgent,
    providerOwner: null,
    requesterOwner: null,
    input: null,
    startedAt,
    endedAt: startedAt + 10,
  }
}

export function team(id: string): Team {
  return {
    id,
    name: id,
    githubOrg: id,
    registryRepo: `github.com/${id}/nandi-registry`,
    members: [],
    createdAt: 0,
    source: { kind: 'git', identifier: id },
  }
}

export function trustRule(
  requester: string,
  providerAgent: string,
  policy: TrustRule['policy']
): TrustRule {
  return { requester, providerAgent, policy, updatedAt: 0 }
}

export const ok = (data: unknown) =>
  vi.fn().mockResolvedValue({ status: 'ok', data })

export const notImplemented = () =>
  vi.fn().mockResolvedValue({ status: 'error', error: 'not implemented' })
