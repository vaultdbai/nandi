/**
 * The "Make this Nandi reachable" checklist (`buildReachability`) from what
 * the app already knows: the relay, the gateway, agents, trust, waiting
 * requests and the directory. No backend of its own.
 */
import { useState } from 'react'
import { useNow } from '@/components/agents/task-utils'
import { buildReachability } from '@/components/network/reachability-model'
import { useAgentOverviews } from '@/hooks/useAgents'
import {
  useGatewayEvents,
  useGatewayManifest,
  useGatewayStatus,
} from '@/hooks/useGateway'
import { useGatewayHolds } from '@/hooks/useInbox'
import { useNetworkSnapshot } from '@/hooks/useNetwork'
import { useRelay, useRelayInstances } from '@/hooks/useRelay'
import { useTrustRequests } from '@/hooks/useTrustRequests'
import { dismiss, isDismissed } from '@/lib/dismissals'

/** The owner chose "no agent takes requests by name" (row 4) */
export const DIRECT_REQUESTS_DISMISSED = 'network.directRequests'

export function useReachability() {
  useGatewayEvents()
  const now = useNow(true, 10_000)
  const { status: relay } = useRelay()
  const gateway = useGatewayStatus()
  const manifest = useGatewayManifest()
  const agents = useAgentOverviews()
  const holds = useGatewayHolds()
  const trust = useTrustRequests()
  const snapshot = useNetworkSnapshot()
  const connected = relay?.connected === true
  const instances = useRelayInstances(connected)
  const [directDismissed, setDirectDismissed] = useState(() =>
    isDismissed(DIRECT_REQUESTS_DISMISSED)
  )
  // When "Make reachable" ran here, for row 3's grace period
  const [listedAt, setListedAt] = useState<number | null>(null)

  const rows = buildReachability({
    relay,
    gateway: gateway.data,
    agents: agents.data,
    directDismissed,
    trustedPeople: manifest.data?.federation.trustedPeers.length ?? 0,
    pendingTrust: trust.data?.length ?? 0,
    waitingApprovals: (holds.data ?? []).filter(h => h.status === 'pending')
      .length,
    members: snapshot.data?.members.length,
    // A relay behind a secret doesn't list its Nandis
    instances: instances.isError ? null : instances.data?.length,
    listedAt,
    now,
  })

  return {
    rows,
    relay,
    gateway: gateway.data,
    agents: agents.data ?? [],
    markListed: () => setListedAt(Date.now()),
    dismissDirect: () => {
      dismiss(DIRECT_REQUESTS_DISMISSED)
      setDirectDismissed(true)
    },
  }
}
