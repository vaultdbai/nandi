/**
 * The Inbox (`buildInbox`) from what's pending now: tool approvals,
 * workflow step approvals, escalated questions, manual steps, a missing
 * model, work assigned to the user, the intake queue (work from outside
 * waiting for the user's OK), and recent failures not dismissed.
 * Kept live by the events of each source; approvals by `agent://approval_*`.
 */
import { useEffect } from 'react'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useNow } from '@/components/agents/task-utils'
import { buildInbox, inboxBadgeCount } from '@/components/inbox/inbox-model'
import { useChiefOfStaffDigest } from '@/hooks/useChiefOfStaffDigest'
import { usePendingClarifications } from '@/hooks/useClarifications'
import { useDefaultModel } from '@/hooks/useLlmKeyStatus'
import { useTrustRequests } from '@/hooks/useTrustRequests'
import { useManualSteps, useWork, workKeys } from '@/hooks/useWork'
import { client } from '@/lib/client'
import { useAgents } from '@/hooks/useAgents'
import { unwrapResult } from '@/lib/tauri-bindings'

export const inboxKeys = {
  approvals: ['approvals', 'pending'] as const,
  holds: ['gateway', 'holds'] as const,
  assignments: ['gateway', 'assignments'] as const,
  dismissed: [...workKeys.all, 'dismissed'] as const,
  returns: ['delegation', 'returns'] as const,
}

const APPROVAL_EVENTS = new Set([
  'agent://approval_request',
  'agent://approval_resolved',
  'nandi://resync',
])

/** Every agent's pending tool approvals (the agent store has one agent's) */
export function useAllPendingApprovals() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (APPROVAL_EVENTS.has(event.event)) {
        void queryClient.invalidateQueries({ queryKey: inboxKeys.approvals })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return useQuery({
    queryKey: inboxKeys.approvals,
    queryFn: async () => unwrapResult(await client.listPendingApprovals()),
  })
}

const HOLD_EVENTS = new Set([
  'gateway://hold_changed',
  'task://state_changed',
  'nandi://resync',
])

/** Other Nandis' requests waiting for the owner's approval (task 5) */
export function useGatewayHolds() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (HOLD_EVENTS.has(event.event)) {
        void queryClient.invalidateQueries({ queryKey: inboxKeys.holds })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return useQuery({
    queryKey: inboxKeys.holds,
    queryFn: async () => unwrapResult(await client.listGatewayHolds()),
  })
}

/** Approve (it goes on) or decline (it's refused) a held request */
export function useDecideGatewayHold() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      taskId,
      approve,
      note = null,
    }: {
      taskId: string
      approve: boolean
      /** Goes back to them: a refusal's reason, a condition */
      note?: string | null
    }) => unwrapResult(await client.decideGatewayHold(taskId, approve, note)),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: inboxKeys.holds }),
  })
}

const ASSIGNMENT_EVENTS = new Set([
  'gateway://assignment_changed',
  'task://state_changed',
  'nandi://resync',
])

/** Work other people assigned to the owner, open until Done or Decline */
export function useGatewayAssignments() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (ASSIGNMENT_EVENTS.has(event.event)) {
        void queryClient.invalidateQueries({
          queryKey: inboxKeys.assignments,
        })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return useQuery({
    queryKey: inboxKeys.assignments,
    queryFn: async () => unwrapResult(await client.listGatewayAssignments()),
  })
}

/** Done (the requester gets the note) or Decline an assignment */
export function useCloseGatewayAssignment() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      taskId,
      done,
      note,
    }: {
      taskId: string
      done: boolean
      note: string | null
    }) => unwrapResult(await client.closeGatewayAssignment(taskId, done, note)),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: inboxKeys.assignments }),
  })
}

const RETURN_EVENTS = new Set(['request://returned', 'nandi://resync'])

/** Requests the user sent that came back while nobody watched */
export function useRequestReturns() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (RETURN_EVENTS.has(event.event)) {
        void queryClient.invalidateQueries({ queryKey: inboxKeys.returns })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])

  return useQuery({
    queryKey: inboxKeys.returns,
    queryFn: async () => unwrapResult(await client.listRequestReturns()),
  })
}

/** Takes a returned request out of the Inbox */
export function useDismissRequestReturn() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (taskId: string) =>
      unwrapResult(await client.dismissRequestReturn(taskId)),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: inboxKeys.returns }),
  })
}

function useDismissedWork() {
  return useQuery({
    queryKey: inboxKeys.dismissed,
    queryFn: async () => unwrapResult(await client.listDismissedWork()),
  })
}

/** Hides a failure from the Inbox; it stays in Recent and history */
export function useDismissWork() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (id: string) =>
      unwrapResult(await client.dismissWork(id)),
    onSettled: () =>
      queryClient.invalidateQueries({ queryKey: inboxKeys.dismissed }),
  })
}

export function useInbox() {
  // Deadlines move items out (and reorder them) without an event
  const now = useNow(true, 30_000)
  const approvals = useAllPendingApprovals()
  const clarifications = usePendingClarifications()
  const manualSteps = useManualSteps()
  const holds = useGatewayHolds()
  const assignments = useGatewayAssignments()
  const trustRequests = useTrustRequests()
  const work = useWork(null)
  const dismissed = useDismissedWork()
  const returns = useRequestReturns()
  const digest = useChiefOfStaffDigest()
  const model = useDefaultModel()
  // A query, not the agent store: the companion's webview has no store sync
  const { data: agents = [] } = useAgents()
  const names = new Map(agents.map(agent => [agent.id, agent.name]))

  const items = buildInbox({
    now,
    noModel: model.data?.effectiveModel === null,
    approvals: approvals.data ?? [],
    clarifications: clarifications.data ?? [],
    manualSteps: manualSteps.data ?? [],
    holds: holds.data ?? [],
    assignments: assignments.data ?? [],
    trustRequests: trustRequests.data ?? [],
    work: work.data ?? [],
    dismissed: dismissed.data ?? [],
    returns: returns.data ?? [],
    queue: digest.data?.queue ?? [],
    agentName: id => names.get(id) ?? id,
  })

  return {
    items,
    count: inboxBadgeCount(items),
    isPending: approvals.isPending || work.isPending,
  }
}
