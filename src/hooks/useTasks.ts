/**
 * The task lifecycle (delegated tasks we send and serve), kept live by
 * `task://*`: a state change refreshes the task's queries, and progress
 * lands in its own cache entry (`useTaskProgress`).
 */
import { useEffect } from 'react'
import {
  useMutation,
  useQuery,
  useQueryClient,
  type QueryClient,
} from '@tanstack/react-query'
import { client, type NandiEvent } from '@/lib/client'
import type {
  JsonValue,
  PendingClarification,
  TaskEvent,
  TaskFilter,
  TaskMessage,
  TaskRecord,
  TaskState,
  TaskStatusReport,
} from '@/lib/bindings'
import { unwrapResult } from '@/lib/tauri-bindings'

/** Payload of `task://state_changed` (`services::a2a::task::TaskStateChanged`) */
export interface TaskStateChangedPayload {
  task: TaskRecord
  event: TaskEvent
}

/** Payload of `task://progress` (`services::a2a::task::TaskProgress`) */
export interface TaskProgressPayload {
  taskId: string
  /** 0–100, when the provider knows */
  percent: number | null
  note: string | null
  at: number
}

/** Everything task-related lives under `['tasks']` */
export const taskQueryKeys = {
  all: ['tasks'] as const,
  lists: ['tasks', 'list'] as const,
  list: (filter: TaskFilter | null) => ['tasks', 'list', filter] as const,
  detail: (taskId: string) => ['tasks', 'detail', taskId] as const,
  events: (taskId: string) => ['tasks', 'events', taskId] as const,
  messages: (taskId: string) => ['tasks', 'messages', taskId] as const,
  progress: (taskId: string) => ['tasks', 'progress', taskId] as const,
  pendingClarifications: ['tasks', 'pending-clarifications'] as const,
  reassignCandidates: (taskId: string) =>
    ['tasks', 'reassign-candidates', taskId] as const,
}

const TERMINAL_STATES: ReadonlySet<TaskState> = new Set<TaskState>([
  'completed',
  'failed',
  'rejected',
  'cancelled',
  'timedOut',
  'abandoned',
  'reassigned',
])

/** Mirrors `TaskState::is_terminal` */
export const isTerminalState = (state: TaskState): boolean =>
  TERMINAL_STATES.has(state)

/** Applies one core event to the task caches */
export function applyTaskEvent(queryClient: QueryClient, event: NandiEvent) {
  switch (event.event) {
    case 'task://state_changed': {
      const payload = event.payload as TaskStateChangedPayload | null
      const taskId = payload?.task?.id
      if (taskId) {
        queryClient.setQueryData(taskQueryKeys.detail(taskId), payload.task)
        void queryClient.invalidateQueries({
          queryKey: taskQueryKeys.events(taskId),
        })
        void queryClient.invalidateQueries({
          queryKey: taskQueryKeys.messages(taskId),
        })
      }
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.lists })
      break
    }
    case 'task://progress': {
      const payload = event.payload as TaskProgressPayload | null
      if (!payload?.taskId) break
      queryClient.setQueryData(taskQueryKeys.progress(payload.taskId), payload)
      void queryClient.invalidateQueries({
        queryKey: taskQueryKeys.messages(payload.taskId),
      })
      break
    }
    case 'task://reassigned':
    case 'task://timed_out':
    case 'task://clarification_requested':
    case 'task://clarification_answered':
    case 'nandi://resync':
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.all })
      break
  }
}

/**
 * One `client.subscribe` per query client, however many task hooks are
 * mounted (ref-counted).
 */
const liveSubscriptions = new Map<
  QueryClient,
  { count: number; unsubscribe: () => void }
>()

function useTaskLiveUpdates() {
  const queryClient = useQueryClient()
  useEffect(() => {
    const existing = liveSubscriptions.get(queryClient)
    if (existing) {
      existing.count += 1
    } else {
      const subscription = client.subscribe(event =>
        applyTaskEvent(queryClient, event)
      )
      liveSubscriptions.set(queryClient, {
        count: 1,
        unsubscribe: subscription.unsubscribe,
      })
    }
    return () => {
      const entry = liveSubscriptions.get(queryClient)
      if (!entry) return
      entry.count -= 1
      if (entry.count === 0) {
        entry.unsubscribe()
        liveSubscriptions.delete(queryClient)
      }
    }
  }, [queryClient])
}

/** Tasks matching `filter` (`null`: the most recent, any role) */
export function useTasks(filter: TaskFilter | null) {
  useTaskLiveUpdates()
  return useQuery<TaskRecord[]>({
    queryKey: taskQueryKeys.list(filter),
    queryFn: async () => unwrapResult(await client.listTasks(filter)),
    retry: false,
  })
}

export function useTask(taskId: string) {
  useTaskLiveUpdates()
  return useQuery<TaskRecord>({
    queryKey: taskQueryKeys.detail(taskId),
    queryFn: async () => unwrapResult(await client.getTask(taskId)),
    retry: false,
  })
}

/** The task's state transitions, oldest first */
export function useTaskEvents(taskId: string) {
  useTaskLiveUpdates()
  return useQuery<TaskEvent[]>({
    queryKey: taskQueryKeys.events(taskId),
    queryFn: async () => unwrapResult(await client.getTaskEvents(taskId)),
    retry: false,
  })
}

/** Progress, status, clarification, … traffic about the task */
export function useTaskMessages(taskId: string) {
  useTaskLiveUpdates()
  return useQuery<TaskMessage[]>({
    queryKey: taskQueryKeys.messages(taskId),
    queryFn: async () => unwrapResult(await client.getTaskMessages(taskId)),
    retry: false,
  })
}

/** The newest `progress` message as a `task://progress` payload, or `null` */
export function latestProgress(
  taskId: string,
  messages: TaskMessage[]
): TaskProgressPayload | null {
  const message = [...messages].reverse().find(m => m.kind === 'progress')
  if (!message) return null
  const payload: Partial<Record<string, JsonValue>> =
    message.payload !== null &&
    typeof message.payload === 'object' &&
    !Array.isArray(message.payload)
      ? message.payload
      : {}
  return {
    taskId,
    percent: typeof payload.percent === 'number' ? payload.percent : null,
    note: typeof payload.note === 'string' ? payload.note : null,
    at: message.at,
  }
}

/**
 * The latest `task://progress` for the task, or `null`. Before an event
 * arrives (a remount, a restart) it falls back to the newest `progress`
 * message, so the bar doesn't vanish when the view is reopened.
 */
export function useTaskProgress(taskId: string) {
  useTaskLiveUpdates()
  const queryClient = useQueryClient()
  return useQuery<TaskProgressPayload | null>({
    queryKey: taskQueryKeys.progress(taskId),
    queryFn: async () => {
      const cached = queryClient.getQueryData<TaskProgressPayload | null>(
        taskQueryKeys.progress(taskId)
      )
      if (cached) return cached
      try {
        const result = await client.getTaskMessages(taskId)
        return result.status === 'ok'
          ? latestProgress(taskId, result.data)
          : null
      } catch {
        return null
      }
    },
    staleTime: Infinity,
  })
}

/**
 * Clarifications waiting on an answer, optionally only one task's. Read
 * here rather than through `useClarifications` so the status panels work
 * on their own; refreshed by `task://clarification_*`.
 */
export function useTaskPendingClarifications(taskId?: string) {
  useTaskLiveUpdates()
  return useQuery<PendingClarification[], unknown, PendingClarification[]>({
    queryKey: taskQueryKeys.pendingClarifications,
    queryFn: async () => unwrapResult(await client.getPendingClarifications()),
    select: all =>
      taskId === undefined ? all : all.filter(c => c.taskId === taskId),
    retry: false,
  })
}

/** Cancels a task (requester side); the provider is told over the relay */
export function useCancelTask() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async ({
      taskId,
      reason = null,
    }: {
      taskId: string
      reason?: string | null
    }) => unwrapResult(await client.cancelTask(taskId, reason)),
    onSuccess: task => {
      queryClient.setQueryData(taskQueryKeys.detail(task.id), task)
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.lists })
    },
  })
}

/** States a provider can hand a task on from (mirrors `reassign::reassignable`) */
export const isReassignable = (state: TaskState): boolean =>
  state === 'delivered' ||
  state === 'acknowledged' ||
  state === 'working' ||
  state === 'inputRequired'

/** Online teammates who serve the task's capability (provider side) */
export function useReassignCandidates(taskId: string, enabled: boolean) {
  useTaskLiveUpdates()
  return useQuery({
    queryKey: taskQueryKeys.reassignCandidates(taskId),
    queryFn: async () =>
      unwrapResult(await client.listReassignCandidates(taskId)),
    enabled,
    staleTime: 15_000,
  })
}

/** Hands a task we're serving to a teammate */
export function useReassignTask() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: async (request: {
      taskId: string
      newProviderInstance: string
      reason?: string | null
    }) =>
      unwrapResult(
        await client.reassignTask({
          taskId: request.taskId,
          newProviderInstance: request.newProviderInstance,
          reason: request.reason ?? null,
        })
      ),
    onSuccess: task => {
      queryClient.setQueryData(taskQueryKeys.detail(task.id), task)
      void queryClient.invalidateQueries({ queryKey: taskQueryKeys.lists })
    },
  })
}

/** Asks the provider how the task is going (falls back to the local row) */
export function useQueryTaskStatus() {
  return useMutation<TaskStatusReport, unknown, string>({
    mutationFn: async taskId =>
      unwrapResult(await client.queryTaskStatus(taskId)),
  })
}

/** `unwrapResult` throws the error string (or, in tests, the `Result`) */
export function taskErrorMessage(error: unknown): string {
  if (typeof error === 'string') return error
  if (error instanceof Error) return error.message
  if (error && typeof error === 'object' && 'error' in error) {
    return String((error as { error: unknown }).error)
  }
  return String(error)
}
