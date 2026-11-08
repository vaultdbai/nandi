/** Test fixtures for the Network tabs */
import type {
  CallerPolicy,
  GatewayManifest,
  GatewayStatus,
  RelayStatus,
  TaskRecord,
  TrustRequest,
} from '@/lib/bindings'

export const manifestFixture = (
  change: Partial<GatewayManifest> = {}
): GatewayManifest => ({
  name: "Ada's Nandi",
  displayName: 'Ada',
  description: 'My Nandi agents.',
  owner: { githubHandle: 'ada', displayName: 'Ada' },
  userSlug: 'ada',
  responsibilities: [],
  availability: { status: 'available', until: null, delegateTo: null },
  relay: { enabled: true, discoverable: true, shareResponsibilities: true },
  policy: {
    maxConcurrent: 3,
    rateLimitPerHour: 30,
    requireApproval: [],
    expose: [],
    hide: [],
  },
  federation: { enabled: false, trustedPeers: [] },
  routing: { preferences: {} },
  body: 'I help with research papers.',
  ...change,
})

export const gatewayStatusFixture = (
  change: Partial<GatewayStatus> = {}
): GatewayStatus => ({
  enabled: true,
  registered: true,
  discoverable: true,
  card: null,
  activeTasks: 0,
  queuedTasks: 0,
  lastHeartbeatAt: Date.now(),
  ...change,
})

export const relayStatusFixture = (
  change: Partial<RelayStatus> = {}
): RelayStatus => ({
  connected: true,
  state: 'connected',
  reason: null,
  instanceId: 'ada-laptop-1a2b3c4d',
  relayUrl: 'wss://relay.test/relay',
  relayUrlSource: 'config',
  agentsOnline: 3,
  instancesOnline: 2,
  lastConnectedAt: Date.now(),
  lastHeartbeatAt: Date.now(),
  ...change,
})

export const callerFixture = (
  change: Partial<CallerPolicy> = {}
): CallerPolicy => ({
  callerInstance: 'bob-laptop-9f8e',
  callerOwner: '',
  allowedCapabilities: [],
  maxConcurrent: 1,
  monthlyTaskBudget: null,
  disabled: false,
  createdAt: 1,
  ...change,
})

export const providerTaskFixture = (
  id: string,
  state: TaskRecord['state'],
  change: Partial<TaskRecord> = {}
): TaskRecord => ({
  id,
  requesterInstance: 'bob-laptop-9f8e',
  requesterAgent: 'bob-laptop-9f8e/writer',
  providerInstance: 'ada-laptop-1a2b3c4d',
  providerAgent: 'papers',
  capability: 'summarize',
  input: null,
  state,
  etaSeconds: null,
  deadline: null,
  priority: 0,
  createdAt: Date.now(),
  updatedAt: Date.now(),
  acknowledgedAt: null,
  completedAt: null,
  error: null,
  result: null,
  parentTaskId: null,
  sourceKind: 'direct',
  attemptNumber: 1,
  previousAttemptId: null,
  ...change,
})

export const trustRequestFixture = (
  change: Partial<TrustRequest> = {}
): TrustRequest => ({
  instanceId: 'sam-desktop-77aa',
  memberName: 'Sam K.',
  handle: 'sam',
  agentId: 'papers',
  agentName: 'Research assistant',
  capability: 'summarize-paper',
  attempts: 3,
  firstAt: Date.now() - 600_000,
  lastAt: Date.now() - 300_000,
  lastTaskId: 'task-sam',
  ...change,
})
