/** Builders for the capability views' tests (Rust-shaped data). */
import type {
  AmbiguityCluster,
  CapabilityDefinition,
  CapabilityEvidence,
  CapabilityListing,
  CapabilityProviderStats,
  NetworkMember,
} from '@/lib/bindings'

export function definition(
  id: string,
  overrides: Partial<CapabilityDefinition> = {}
): CapabilityDefinition {
  return {
    id,
    name: id,
    description: `Does ${id}`,
    version: '1.0.0',
    owner: { kind: 'user', id: 'me', displayName: 'Me', transferable: true },
    policy: {
      visibility: 'network',
      maxConcurrentPerProvider: 1,
      totalMaxConcurrent: 0,
      slaSeconds: 0,
      approvalRequired: false,
      cost: null,
      autoReassign: 'never',
    },
    schemas: { input: null, output: null },
    requiredTools: ['builtin.shell'],
    optionalTools: [],
    tags: [],
    createdAt: null,
    updatedAt: null,
    body: '',
    ...overrides,
  }
}

export function listing(
  id: string,
  overrides: Partial<CapabilityListing> = {},
  definitionOverrides: Partial<CapabilityDefinition> = {}
): CapabilityListing {
  return {
    definition: definition(id, definitionOverrides),
    source: 'local',
    registryOrigin: null,
    owned: false,
    provided: false,
    used: false,
    localProviders: 0,
    onlineProviders: 0,
    successRate: null,
    invocations: 0,
    missingTools: [],
    ...overrides,
  }
}

export function evidence(
  overrides: Partial<CapabilityEvidence> = {}
): CapabilityEvidence {
  return {
    capabilityId: 'cap',
    capabilityVersion: '1.0.0',
    instanceId: 'i-me',
    providerKind: 'agent',
    providerId: 'reviewer',
    windowDays: 30,
    count: 0,
    successCount: 0,
    failedCount: 0,
    timeoutCount: 0,
    p50Ms: null,
    p95Ms: null,
    tokensInTotal: 0,
    tokensOutTotal: 0,
    costUsdTotal: null,
    clarificationRoundsAvg: null,
    escalationRate: null,
    userRatingAvg: null,
    lastObservedAt: null,
    ...overrides,
  }
}

export function providerStats(
  providerId: string,
  overrides: {
    local?: boolean
    online?: boolean
    load?: number
    kind?: 'agent' | 'workflow'
    instanceId?: string
    evidence?: Partial<CapabilityEvidence>
  } = {}
): CapabilityProviderStats {
  const instanceId = overrides.instanceId ?? 'i-me'
  const kind = overrides.kind ?? 'agent'
  return {
    provider: {
      capabilityId: 'cap',
      capabilityVersion: '1.0.0',
      instanceId,
      providerKind: kind,
      providerId,
      displayName: providerId,
      local: overrides.local ?? true,
      enabled: true,
      maxConcurrent: 2,
      providedTools: ['builtin.shell'],
      registeredAt: null,
    },
    online: overrides.online ?? true,
    load: overrides.load ?? 0,
    evidence: evidence({
      instanceId,
      providerId,
      providerKind: kind,
      ...overrides.evidence,
    }),
  }
}

export function cluster(
  id: string,
  overrides: Partial<AmbiguityCluster> = {}
): AmbiguityCluster {
  return {
    id,
    capabilityId: 'cap',
    clusterLabel: 'Which Python version?',
    sampleQuestions: ['Which Python version?', 'Python 3.11 or 3.12?'],
    occurrenceCount: 18,
    firstSeenAt: null,
    lastSeenAt: null,
    promotedToSchema: false,
    ...overrides,
  }
}

export function networkMember(
  instanceId: string,
  displayName: string,
  handle: string | null
): NetworkMember {
  return {
    instanceId,
    displayName,
    avatarUrl: null,
    online: true,
    lastSeen: null,
    handle,
    gateway: {
      instanceId,
      endpoint: '',
      displayName,
      description: '',
      capabilities: [],
      acceptsDelegation: true,
      maxConcurrent: 1,
      currentLoad: 0,
      owner: { githubHandle: handle ?? '', displayName },
      federationEnabled: false,
      gatewayVersion: 1,
      agentCount: 0,
      body: null,
    },
  }
}
