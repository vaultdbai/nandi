/**
 * Pure rules for asking another member's Nandi: their gateway (the
 * `<instance>/gateway` card) takes a natural-language request and routes it
 * to one of their agents.
 */
import type { JsonValue } from '@/lib/bindings'

/** The capability a gateway serves natural-language requests under */
export const GATEWAY_CAPABILITY = 'gateway'

/** Why a member can't be asked right now (`available`: they can) */
export type AskAvailability =
  'available' | 'notConnected' | 'offline' | 'notAccepting'

export function askAvailability(member: {
  online: boolean
  gateway: { acceptsDelegation: boolean } | null
}): AskAvailability {
  if (!member.gateway) return 'notConnected'
  if (!member.online) return 'offline'
  if (!member.gateway.acceptsDelegation) return 'notAccepting'
  return 'available'
}

/** The card ID of a member's gateway */
export function gatewayTarget(instanceId: string): string {
  return `${instanceId}/${GATEWAY_CAPABILITY}`
}

/** The `delegateTask` arguments for asking `instanceId` in plain words */
export function gatewayRequest(
  instanceId: string,
  text: string
): { capability: string; input: JsonValue; target: string } {
  return {
    capability: GATEWAY_CAPABILITY,
    input: { text: text.trim() },
    target: gatewayTarget(instanceId),
  }
}
