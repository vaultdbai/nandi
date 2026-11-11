/**
 * The Inbox (`buildInbox`) from what's pending now: tool approvals,
 * workflow step approvals, escalated questions, manual steps, a missing
 * model, work assigned to the user, the intake queue (work from outside
 * waiting for the user's OK), and recent failures not dismissed.
 * Kept live by the events of each source; approvals by `agent://approval_*`.
 */
import { useNow } from '@/components/agents/task-utils'
import { buildInbox, inboxBadgeCount } from '@/components/inbox/inbox-model'
import { useChiefOfStaffDigest } from '@/hooks/useChiefOfStaffDigest'
import { usePendingClarifications } from '@/hooks/useClarifications'
import { useKnowMe } from '@/hooks/useKnowMe'
import { useDefaultModel } from '@/hooks/useLlmKeyStatus'
import { useTrustRequests } from '@/hooks/useTrustRequests'
import { useManualSteps, useWork, workKeys } from '@/hooks/useWork'
import { client, type NandiEventName } from '@/lib/client'
import { useInvalidateOn } from '@/hooks/useInvalidateOn'
import { useAgents } from '@/hooks/useAgents'
import { useClientMutation, useClientQuery } from '@/hooks/useClientQuery'

export const inboxKeys = {
  approvals: ['approvals', 'pending'] as const,
  holds: ['gateway', 'holds'] as const,
  assignments: ['gateway', 'assignments'] as const,
  dismissed: [...workKeys.all, 'dismissed'] as const,
  returns: ['delegation', 'returns'] as const,
}

const APPROVAL_EVENTS: readonly NandiEventName[] = [
  'agent://approval_request',
  'agent://approval_resolved',
]

/** Every agent's pending tool approvals (the agent store has one agent's) */
export function useAllPendingApprovals() {
  useInvalidateOn(APPROVAL_EVENTS, inboxKeys.approvals)

  return useClientQuery(inboxKeys.approvals, () =>
    client.listPendingApprovals()
  )
}

const HOLD_EVENTS: readonly NandiEventName[] = [
  'gateway://hold_changed',
  'task://state_changed',
]

/** Other Nandis' requests waiting for the owner's approval (task 5) */
export function useGatewayHolds() {
  useInvalidateOn(HOLD_EVENTS, inboxKeys.holds)

  return useClientQuery(inboxKeys.holds, () => client.listGatewayHolds())
}

/** Approve (it goes on) or decline (it's refused) a held request */
export function useDecideGatewayHold() {
  return useClientMutation(
    ({
      taskId,
      approve,
      note = null,
    }: {
      taskId: string
      approve: boolean
      /** Goes back to them: a refusal's reason, a condition */
      note?: string | null
    }) => client.decideGatewayHold(taskId, approve, note),
    { invalidates: inboxKeys.holds }
  )
}

const ASSIGNMENT_EVENTS: readonly NandiEventName[] = [
  'gateway://assignment_changed',
  'task://state_changed',
]

/** Work other people assigned to the owner, open until Done or Decline */
export function useGatewayAssignments() {
  useInvalidateOn(ASSIGNMENT_EVENTS, inboxKeys.assignments)

  return useClientQuery(inboxKeys.assignments, () =>
    client.listGatewayAssignments()
  )
}

/** Done (the requester gets the note) or Decline an assignment */
export function useCloseGatewayAssignment() {
  return useClientMutation(
    ({
      taskId,
      done,
      note,
    }: {
      taskId: string
      done: boolean
      note: string | null
    }) => client.closeGatewayAssignment(taskId, done, note),
    { invalidates: inboxKeys.assignments }
  )
}

const RETURN_EVENTS: readonly NandiEventName[] = ['request://returned']

/** Requests the user sent that came back while nobody watched */
export function useRequestReturns() {
  useInvalidateOn(RETURN_EVENTS, inboxKeys.returns)

  return useClientQuery(inboxKeys.returns, () => client.listRequestReturns())
}

/** Takes a returned request out of the Inbox */
export function useDismissRequestReturn() {
  return useClientMutation(
    (taskId: string) => client.dismissRequestReturn(taskId),
    {
      invalidates: inboxKeys.returns,
    }
  )
}

function useDismissedWork() {
  return useClientQuery(inboxKeys.dismissed, () => client.listDismissedWork())
}

/** Hides a failure from the Inbox; it stays in Recent and history */
export function useDismissWork() {
  return useClientMutation((id: string) => client.dismissWork(id), {
    invalidates: inboxKeys.dismissed,
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
  const knowMe = useKnowMe()
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
    knowMe: knowMe.data ?? null,
    agentName: id => names.get(id) ?? id,
  })

  return {
    items,
    count: inboxBadgeCount(items),
    isPending: approvals.isPending || work.isPending,
  }
}
