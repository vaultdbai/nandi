/** Test fixtures for the Network tabs */
import type {
  GatewayManifest,
  GatewayStatus,
  RelayStatus,
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
