import { describe, expect, it } from 'vitest'
import type { GatewayStatus, RelayStatus } from '@/lib/bindings'
import {
  buildReachability,
  CONFIRM_GRACE_MS,
  doneCount,
  HEARTBEAT_FRESH_MS,
  healthLabel,
  networkHealth,
  type ReachabilityInput,
  type RowKey,
} from './reachability-model'

const NOW = 1_000_000_000

const relay = (overrides: Partial<RelayStatus> = {}): RelayStatus => ({
  connected: true,
  state: 'connected',
  reason: null,
  instanceId: 'desktop-me',
  relayUrl: 'ws://relay.test/relay',
  relayUrlSource: 'config',
  agentsOnline: 3,
  instancesOnline: 4,
  lastConnectedAt: NOW - 60_000,
  lastHeartbeatAt: NOW - 5_000,
  ...overrides,
})

const gateway = (overrides: Partial<GatewayStatus> = {}): GatewayStatus => ({
  enabled: true,
  registered: true,
  discoverable: true,
  card: null,
  activeTasks: 0,
  queuedTasks: 0,
  lastHeartbeatAt: NOW - 10_000,
  ...overrides,
})

const input = (
  overrides: Partial<ReachabilityInput> = {}
): ReachabilityInput => ({
  relay: relay(),
  gateway: gateway(),
  agents: [{ acceptsRequests: true }, { acceptsRequests: false }],
  directDismissed: false,
  trustedPeople: 2,
  pendingTrust: 0,
  waitingApprovals: 0,
  members: 3,
  instances: 4,
  listedAt: null,
  now: NOW,
  ...overrides,
})

const row = (rows: ReturnType<typeof buildReachability>, key: RowKey) => {
  const found = rows.find(r => r.key === key)
  if (!found) throw new Error(`no row ${key}`)
  return found
}

describe('buildReachability', () => {
  it('passes every row on a fully reachable Nandi', () => {
    const rows = buildReachability(input())
    expect(rows.map(r => r.key)).toEqual([
      'relay',
      'listed',
      'confirmed',
      'direct',
      'callers',
      'approvals',
      'visibility',
    ])
    expect(rows.every(r => r.status === 'pass')).toBe(true)
    expect(doneCount(rows)).toBe(7)
  })

  it('row 1: no relay connection, and the rows after it wait', () => {
    const rows = buildReachability(
      input({ relay: relay({ connected: false, state: 'disconnected' }) })
    )
    expect(row(rows, 'relay').status).toBe('fail')
    expect(row(rows, 'listed').status).toBe('blocked')
    expect(row(rows, 'confirmed').status).toBe('blocked')
    expect(row(rows, 'visibility').status).toBe('blocked')
  })

  it('row 2: hidden until the gateway is on and listed', () => {
    for (const status of [
      gateway({ enabled: false }),
      gateway({ discoverable: false }),
    ]) {
      const rows = buildReachability(input({ gateway: status }))
      expect(row(rows, 'listed').status).toBe('fail')
      expect(row(rows, 'confirmed').status).toBe('blocked')
    }
  })

  it('row 3: needs a fresh heartbeat; checks briefly after a fix, then offers a test', () => {
    const stale = gateway({ lastHeartbeatAt: NOW - HEARTBEAT_FRESH_MS - 1 })
    expect(
      row(buildReachability(input({ gateway: stale })), 'confirmed').status
    ).toBe('fail')
    const pending = gateway({ registered: false })
    expect(
      row(
        buildReachability(input({ gateway: pending, listedAt: NOW - 5_000 })),
        'confirmed'
      ).status
    ).toBe('checking')
    expect(
      row(
        buildReachability(
          input({ gateway: pending, listedAt: NOW - CONFIRM_GRACE_MS })
        ),
        'confirmed'
      ).status
    ).toBe('fail')
  })

  it('row 4: a warning without agents taking requests by name, unless chosen', () => {
    const none = [{ acceptsRequests: false }]
    const rows = buildReachability(input({ agents: none }))
    expect(row(rows, 'direct')).toEqual({
      key: 'direct',
      status: 'warn',
      count: 0,
    })
    expect(
      row(
        buildReachability(input({ agents: none, directDismissed: true })),
        'direct'
      ).status
    ).toBe('pass')
    expect(row(buildReachability(input()), 'direct').count).toBe(1)
  })

  it('row 5: always a summary, with people waiting for an answer', () => {
    const callers = row(
      buildReachability(input({ trustedPeople: 1, pendingTrust: 2 })),
      'callers'
    )
    expect(callers).toEqual({
      key: 'callers',
      status: 'pass',
      count: 1,
      extra: 2,
    })
  })

  it('row 6: requests waiting for approval need the owner', () => {
    const rows = buildReachability(input({ waitingApprovals: 2 }))
    expect(row(rows, 'approvals')).toEqual({
      key: 'approvals',
      status: 'fail',
      count: 2,
    })
  })

  it('row 7: says how many Nandis are out of sight, or that the relay won’t say', () => {
    const partial = row(
      buildReachability(input({ members: 2, instances: 4 })),
      'visibility'
    )
    expect(partial).toEqual({
      key: 'visibility',
      status: 'info',
      count: 3,
      extra: 1,
    })
    expect(
      row(buildReachability(input({ instances: null })), 'visibility')
    ).toEqual({ key: 'visibility', status: 'info', hidden: true })
    expect(
      row(buildReachability(input({ instances: undefined })), 'visibility')
        .status
    ).toBe('checking')
  })
})

describe('networkHealth', () => {
  it('is nothing without a relay, red on error, green only when reachable', () => {
    expect(networkHealth(relay({ relayUrl: null }), gateway())).toBe('none')
    expect(networkHealth(undefined, undefined)).toBe('none')
    expect(networkHealth(relay({ state: 'error' }), gateway())).toBe('red')
    expect(networkHealth(relay(), gateway())).toBe('green')
    expect(networkHealth(relay(), gateway({ discoverable: false }))).toBe(
      'amber'
    )
    expect(networkHealth(relay(), gateway({ registered: false }))).toBe('amber')
    expect(
      networkHealth(
        relay({ connected: false, state: 'reconnecting' }),
        gateway()
      )
    ).toBe('amber')
  })
})

describe('healthLabel', () => {
  it('says connecting while amber and not connected, else the health', () => {
    const connecting = relay({ connected: false, state: 'connecting' })
    expect(healthLabel(connecting, 'amber')).toBe('connecting')
    expect(healthLabel(relay(), 'amber')).toBe('amber')
    expect(healthLabel(relay(), 'green')).toBe('green')
  })
})
