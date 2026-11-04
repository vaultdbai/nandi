/** Time and naming helpers for showing delegated tasks */
import { useEffect, useState } from 'react'
import type { NetworkSnapshot, TaskRecord } from '@/lib/bindings'
import { USER_REQUESTER } from '@/hooks/useClarifications'
import { useNetworkSnapshot } from '@/hooks/useNetwork'

/** `Date.now()`, re-read every `intervalMs` while `active` */
export function useNow(active = true, intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (!active) return
    const timer = setInterval(() => setNow(Date.now()), intervalMs)
    return () => clearInterval(timer)
  }, [active, intervalMs])
  return now
}

/** When the task started running (acknowledged), else when it was sent */
export const taskStartedAt = (task: TaskRecord): number =>
  task.acknowledgedAt ?? task.createdAt

/** "agent · Priya": the agent, then its member's name (never the device ID) */
export function agentLabel(agent: string, member: string | null): string {
  return member ? `${agent} · ${member}` : agent
}

/** A member's display name from the directory, else `null` */
export function memberName(
  snapshot: NetworkSnapshot | undefined,
  instance: string
): string | null {
  if (!instance || !snapshot) return null
  const member = snapshot.members.find(m => m.instanceId === instance)
  return member?.displayName || (member?.handle ? `@${member.handle}` : null)
}

/** Names instances from the directory: `nameOf(instance)` */
export function useMemberNames() {
  const { data } = useNetworkSnapshot()
  return (instance: string) => memberName(data, instance)
}

/** `agentLabel` with the member named from the directory */
export function useAgentLabel() {
  const nameOf = useMemberNames()
  return (agent: string, instance: string) =>
    agentLabel(agent, nameOf(instance))
}

/** Whether the local user sent the task themselves (no agent asked) */
export const isUserRequester = (agent: string): boolean =>
  agent === USER_REQUESTER

/**
 * The provider member's name when the task is queued (`submitted`) for a
 * member the directory shows offline: the core sends it once they're back.
 */
export function offlineProviderName(
  task: TaskRecord,
  snapshot: NetworkSnapshot | undefined
): string | null {
  if (task.state !== 'submitted' || !snapshot) return null
  const member = snapshot.members.find(
    m => m.instanceId === task.providerInstance
  )
  if (!member || member.online) return null
  return member.displayName || member.instanceId
}

/** `offlineProviderName`, reading the directory only while queued */
export function useOfflineProvider(task: TaskRecord | undefined) {
  const queued = task?.state === 'submitted'
  const { data } = useNetworkSnapshot(queued)
  return task ? offlineProviderName(task, data) : null
}
