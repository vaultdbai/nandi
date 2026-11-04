/** Builders for Rust-shaped data in frontend tests. */

import type {
  AgentInfo,
  AgentSnapshot,
  ChatMessage,
  DashboardSnapshot,
  QueueEntry,
  RunSummary,
} from '@/lib/tauri-bindings'
import type { AgentManifest, PackManifest } from '@/lib/bindings'

export const T0 = Date.UTC(2026, 9, 2, 12, 0, 0)

export function agentInfo(
  id: string,
  name: string,
  overrides: Partial<AgentInfo> = {}
): AgentInfo {
  return {
    id,
    name,
    description: '',
    skillPath: `/home/me/.nandi/agents/${id}/skill.md`,
    status: 'idle',
    currentTask: null,
    origin: { kind: 'local' },
    builtin: false,
    ...overrides,
  }
}

export function run(
  id: string,
  agentId: string,
  overrides: Partial<RunSummary> = {}
): RunSummary {
  return {
    id,
    agentId,
    conversationId: `agent:${agentId}`,
    status: 'running',
    input: 'Do the thing',
    summary: null,
    startedAt: T0,
    endedAt: null,
    inputTokens: 0,
    outputTokens: 0,
    costUsd: null,
    model: null,
    sourceType: 'user',
    sourceId: null,
    sourceLabel: null,
    priority: 10,
    queuedAt: null,
    parentRunId: null,
    compactionSummary: null,
    ...overrides,
  }
}

export const userMessage = (
  id: string,
  content: string,
  runId = 'r1'
): ChatMessage => ({
  kind: 'user',
  id,
  runId,
  createdAt: T0,
  content,
  attachments: [],
})

export const agentMessage = (
  id: string,
  content: string,
  runId = 'r1'
): ChatMessage => ({
  kind: 'agent',
  id,
  runId,
  createdAt: T0,
  content,
})

export const approvalMessage = (
  id: string,
  status: 'pending' | 'approved' | 'rejected' | 'expired' = 'pending',
  runId = 'r1'
): ChatMessage => ({
  kind: 'approval',
  id,
  runId,
  createdAt: T0,
  tool: 'shell',
  args: { cmd: 'wc -w report.md' },
  status,
  expiresAt: null,
})

export function agentSnapshot(
  agent: AgentInfo | null,
  agentId: string,
  overrides: Partial<AgentSnapshot> & {
    messages?: ChatMessage[]
    currentRun?: RunSummary | null
  } = {}
): AgentSnapshot {
  const { messages = [], currentRun = null, ...rest } = overrides
  return {
    agent,
    conversation: { conversationId: `agent:${agentId}`, messages, currentRun },
    pendingApprovals: [],
    recentRuns: [],
    queue: [],
    currentTurn: null,
    recentEvents: [],
    container: { status: 'idle', error: null },
    capabilities: null,
    ...rest,
  }
}

export function queueEntry(
  runId: string,
  agentId: string,
  overrides: Partial<QueueEntry> = {}
): QueueEntry {
  return {
    id: `q-${runId}`,
    agentId,
    runId,
    sourceType: 'user',
    sourceId: null,
    sourceLabel: null,
    priority: 10,
    message: 'Queued request',
    attachments: [],
    createdAt: T0,
    position: 0,
    ...overrides,
  }
}

export function dashboardSnapshot(
  overrides: Partial<DashboardSnapshot> = {}
): DashboardSnapshot {
  return {
    stats: {
      needsAttention: 0,
      runningAgents: 0,
      runsToday: 0,
      failedLastDay: 0,
    },
    triggerStats: { enabled: 0, firedToday: 0 },
    recentTriggerFires: [],
    attentionItems: [],
    recentActivity: [],
    agentHealth: [],
    recentOutputs: [],
    ...overrides,
  }
}

export function packManifest(
  id: string,
  name: string,
  overrides: Partial<PackManifest> = {}
): PackManifest {
  return {
    id,
    name,
    description: `${name} description`,
    publisher: 'vaultdbai',
    repo: `github.com/vaultdbai/nandi-${id}`,
    latestVersion: '1.0.0',
    gitRef: 'v1.0.0',
    tags: [],
    minNandiVersion: '0.1.0',
    agents: [
      { id: `${id}-agent`, name: `${name} Agent`, description: 'Does things' },
    ],
    workflows: [`${id}-flow`],
    suggestedAsks: [],
    capabilities: {
      tools: ['shell', 'read_file'],
      network: ['export.arxiv.org'],
      pilots: [],
      requiresApproval: ['shell'],
      aws: [],
      connectors: [],
    },
    installSizeKb: 24,
    estimatedCostUsd: { minUsd: 0.01, maxUsd: 0.25 },
    signature: null,
    commit: null,
    iconUrl: null,
    readmeUrl: null,
    registry: 'github.com/vaultdbai/nandi-registry',
    compatible: true,
    installedVersion: null,
    verified: false,
    ...overrides,
  }
}

export function agentManifest(
  id: string,
  name: string,
  overrides: Partial<AgentManifest> = {}
): AgentManifest {
  return {
    id,
    name,
    description: `${name} description`,
    publisher: 'alice',
    repo: 'github.com/alice/nandi-agents',
    path: `agents/${id}`,
    latestVersion: '1.2.0',
    gitRef: 'v1.2.0',
    tags: ['research'],
    minNandiVersion: '0.1.0',
    capabilities: {
      tools: ['shell', 'read_file'],
      network: ['arxiv.org'],
      pilots: [],
      requiresApproval: ['shell'],
      aws: [],
      connectors: [],
    },
    signature: null,
    commit: null,
    iconUrl: null,
    readmeUrl: null,
    estimatedCostUsd: null,
    registry: 'github.com/vaultdbai/nandi-registry',
    compatible: true,
    installedVersion: null,
    verified: false,
    ...overrides,
  }
}
