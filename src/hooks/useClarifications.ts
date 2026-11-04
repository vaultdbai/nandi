/**
 * Clarification questions waiting for the user, and answering or
 * escalating them (TanStack Query), kept live by `task://clarification_*`
 * and `task://state_changed` events.
 *
 * Question IDs are `r<round>-q<n>` (the provider's `ask_requester` tool
 * numbers them), so the round can be read off a pending question.
 */
import { useEffect } from 'react'
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from '@tanstack/react-query'
import { client } from '@/lib/client'
import type {
  ClarificationPolicy,
  ClarificationResponse,
  PendingClarification,
} from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'
import { taskQueryKeys } from './useTasks'

/** Answers with a meaning (mirrors `services::a2a::clarification`) */
export const DENY_ANSWER = '__nandi:deny__'
export const ALLOW_ONCE_ANSWER = '__nandi:allow_once__'
export const ALLOW_ALWAYS_ANSWER = '__nandi:allow_always__'

/** Agent ID of questions about the user's own delegations */
export const USER_REQUESTER = '_user'

/** `ClarificationPolicy` default */
export const DEFAULT_MAX_ROUNDS = 2

export const clarificationQueryKeys = {
  all: ['clarification'] as const,
  pending: ['clarification', 'pending'] as const,
  /** A requester agent's policy (its skill.md `clarification:`) */
  policy: (agentId: string) => ['clarification', 'policy', agentId] as const,
}

const INVALIDATING_EVENTS = new Set<string>([
  'task://clarification_requested',
  'task://clarification_answered',
  'task://state_changed',
])

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/** The round of a question ID (`r2-q1` → 2), or `null` */
export function roundOf(questionId: string): number | null {
  const match = /^r(\d+)-q\d+$/.exec(questionId)
  return match ? Number(match[1]) : null
}

/** The response that answers one pending question */
export function answerTo(
  clarification: PendingClarification,
  answer: string
): ClarificationResponse {
  return {
    taskId: clarification.taskId,
    // 0: whichever round is open
    round: roundOf(clarification.question.id) ?? 0,
    answers: { [clarification.question.id]: answer },
  }
}

/** Milliseconds until `deadline` (never negative), or `null` */
export function remainingMs(
  deadline: number | null,
  now: number
): number | null {
  return deadline === null ? null : Math.max(0, deadline - now)
}

/** `m:ss`, or `h:mm:ss` past an hour */
export function formatRemaining(ms: number): string {
  const total = Math.ceil(ms / 1000)
  const hours = Math.floor(total / 3600)
  const minutes = Math.floor((total % 3600) / 60)
  const seconds = String(total % 60).padStart(2, '0')
  return hours > 0
    ? `${hours}:${String(minutes).padStart(2, '0')}:${seconds}`
    : `${minutes}:${seconds}`
}

// ---------------------------------------------------------------------------
// Queries
// ---------------------------------------------------------------------------

/** Keeps clarification queries current from task events */
export function useClarificationEvents() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const subscription = client.subscribe(event => {
      if (INVALIDATING_EVENTS.has(event.event)) {
        void queryClient.invalidateQueries({
          queryKey: clarificationQueryKeys.pending,
        })
      }
    })
    return () => subscription.unsubscribe()
  }, [queryClient])
}

/** Questions waiting for the user, optionally for one task */
export function usePendingClarifications(taskId?: string) {
  useClarificationEvents()
  return useQuery({
    queryKey: clarificationQueryKeys.pending,
    queryFn: async () => unwrapResult(await client.getPendingClarifications()),
    select: (rows: PendingClarification[]) =>
      taskId === undefined ? rows : rows.filter(row => row.taskId === taskId),
  })
}

/**
 * Both caches of pending questions: this module's and the task panels'
 * (`useTaskPendingClarifications`), so an answered card goes away at once
 * rather than on the next `task://clarification_answered`.
 */
function invalidatePending(queryClient: QueryClient) {
  return Promise.all([
    queryClient.invalidateQueries({ queryKey: clarificationQueryKeys.pending }),
    queryClient.invalidateQueries({
      queryKey: taskQueryKeys.pendingClarifications,
    }),
  ])
}

/** Answers questions (text, or one of the special answers) */
export function useRespondToClarification() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (response: ClarificationResponse) =>
      unwrapResult(await client.respondToClarification(response)),
    onSettled: () => invalidatePending(queryClient),
  })
}

/** Sends a question to the user even if the agent would answer it */
export function useEscalateClarification() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      taskId,
      questionId,
    }: {
      taskId: string
      questionId: string
    }) => unwrapResult(await client.escalateClarification(taskId, questionId)),
    onSettled: () => invalidatePending(queryClient),
  })
}

/** A local agent's clarification policy; off for `enabled: false` */
export function useClarificationPolicy(agentId: string, enabled = true) {
  return useQuery({
    queryKey: clarificationQueryKeys.policy(agentId),
    queryFn: async () =>
      unwrapResult(await client.getClarificationPolicy(agentId)),
    enabled: enabled && agentId.length > 0,
  })
}

/** Saves an agent's clarification policy to its skill.md */
export function useSetClarificationPolicy(agentId: string) {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (policy: ClarificationPolicy) =>
      unwrapResult(await client.setClarificationPolicy(agentId, policy)),
    onSuccess: saved => {
      queryClient.setQueryData(clarificationQueryKeys.policy(agentId), saved)
      void queryClient.invalidateQueries({
        queryKey: clarificationQueryKeys.policy(agentId),
      })
    },
  })
}
