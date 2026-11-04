import {
  healthLabel,
  networkHealth,
} from '@/components/network/reachability-model'
import { useGatewayStatus } from '@/hooks/useGateway'
import { useRelay } from '@/hooks/useRelay'

/**
 * The network's health from the relay and gateway (nav dot, header pill),
 * and its words (`label`: a `network.health.*` key)
 */
export function useNetworkHealth() {
  const { status } = useRelay()
  const gateway = useGatewayStatus()
  const health = networkHealth(status, gateway.data)
  return { health, label: `network.health.${healthLabel(status, health)}` }
}
