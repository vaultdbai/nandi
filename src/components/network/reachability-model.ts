/**
 * "Make this Nandi reachable" (network.md §4.2): the seven opt-ins a
 * request from another Nandi goes through, each with whether it passes and
 * the one fix that moves it on. Also the network's health for the nav dot
 * and the page's status pill (§3). Pure, so every row is tested.
 */
import type { GatewayStatus, RelayStatus } from '@/lib/bindings'

/** How long "the relay confirmed it" stays true without a heartbeat */
export const HEARTBEAT_FRESH_MS = 90_000
/** How long row 3 waits after row 2 is fixed before offering a test */
export const CONFIRM_GRACE_MS = 30_000

export type RowKey =
  | 'relay'
  | 'listed'
  | 'confirmed'
  | 'direct'
  | 'callers'
  | 'approvals'
  | 'visibility'

export type RowStatus =
  /** Done */
  | 'pass'
  /** Needs the user (the row's fix button) */
  | 'fail'
  /** Works without it, but something is off (row 4) */
  | 'warn'
  /** Just for information (row 7) */
  | 'info'
  /** Waiting for the relay, briefly */
  | 'checking'
  /** Can't be fixed until an earlier row is */
  | 'blocked'

export interface ReachabilityRow {
  key: RowKey
  status: RowStatus
  /** Agents taking requests by name (row 4); people (row 5); waiting (row 6) */
  count?: number
  /** Row 5: people waiting for a trust answer; row 7: Nandis out of sight */
  extra?: number
  /** Row 7: the relay doesn't say how many Nandis it has */
  hidden?: boolean
}

export interface ReachabilityInput {
  relay: RelayStatus | undefined
  gateway: GatewayStatus | undefined
  /** Local agents, with whether each takes requests addressed to it */
  agents: { acceptsRequests: boolean }[] | undefined
  /** The owner chose "no agent takes requests by name" */
  directDismissed: boolean
  trustedPeople: number
  pendingTrust: number
  /** Incoming requests waiting for the owner's approval */
  waitingApprovals: number
  /** Others in the directory (the snapshot leaves this Nandi out) */
  members: number | undefined
  /** Nandis the relay lists (`null`: it doesn't share its count) */
  instances: number | null | undefined
  /** When row 2 was last fixed here, if just now */
  listedAt: number | null
  now: number
}

export function buildReachability(input: ReachabilityInput): ReachabilityRow[] {
  const { relay, gateway, now } = input
  const connected = relay?.connected === true
  const listed =
    gateway !== undefined && gateway.enabled && gateway.discoverable
  const fresh =
    gateway?.lastHeartbeatAt != null &&
    now - gateway.lastHeartbeatAt <= HEARTBEAT_FRESH_MS
  const confirmed = listed && gateway.registered && fresh
  const takingDirect = (input.agents ?? []).filter(
    a => a.acceptsRequests
  ).length

  const confirmedStatus = (): RowStatus => {
    if (!connected || !listed) return 'blocked'
    if (confirmed) return 'pass'
    const justListed =
      input.listedAt !== null && now - input.listedAt < CONFIRM_GRACE_MS
    return justListed ? 'checking' : 'fail'
  }

  const visibility = (): ReachabilityRow => {
    if (!connected) return { key: 'visibility', status: 'blocked' }
    if (input.instances === null) {
      return { key: 'visibility', status: 'info', hidden: true }
    }
    if (input.instances === undefined || input.members === undefined) {
      return { key: 'visibility', status: 'checking' }
    }
    // The relay counts this Nandi; the directory doesn't
    const seen = input.members + 1
    const missing = Math.max(0, input.instances - seen)
    return {
      key: 'visibility',
      status: missing === 0 ? 'pass' : 'info',
      count: Math.min(seen, input.instances),
      extra: missing,
    }
  }

  return [
    { key: 'relay', status: connected ? 'pass' : 'fail' },
    {
      key: 'listed',
      status: !connected ? 'blocked' : listed ? 'pass' : 'fail',
    },
    { key: 'confirmed', status: confirmedStatus() },
    {
      key: 'direct',
      status: takingDirect > 0 || input.directDismissed ? 'pass' : 'warn',
      count: takingDirect,
    },
    {
      key: 'callers',
      status: 'pass',
      count: input.trustedPeople,
      extra: input.pendingTrust,
    },
    {
      key: 'approvals',
      status: input.waitingApprovals > 0 ? 'fail' : 'pass',
      count: input.waitingApprovals,
    },
    visibility(),
  ]
}

/** "5 of 7 done" */
export function doneCount(rows: ReachabilityRow[]): number {
  return rows.filter(row => row.status === 'pass').length
}

export type NetworkHealth =
  /** No relay saved: networking is off, not broken */
  | 'none'
  /** Connected, listed and confirmed: others can reach this Nandi */
  | 'green'
  /** Connecting, or connected but not listed or confirmed yet */
  | 'amber'
  /** The relay refused or failed */
  | 'red'

/** The nav dot and header pill (network.md §3) */
export function networkHealth(
  relay: RelayStatus | undefined,
  gateway: GatewayStatus | undefined
): NetworkHealth {
  if (!relay?.relayUrl) return 'none'
  if (relay.state === 'error') return 'red'
  const reachable =
    relay.connected &&
    gateway !== undefined &&
    gateway.enabled &&
    gateway.discoverable &&
    gateway.registered
  return reachable ? 'green' : 'amber'
}

/** The dot in words (`network.health.*`): amber says if it's still connecting */
export function healthLabel(
  relay: RelayStatus | undefined,
  health: NetworkHealth
): NetworkHealth | 'connecting' {
  return health === 'amber' && !relay?.connected ? 'connecting' : health
}
