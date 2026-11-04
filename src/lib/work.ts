/**
 * Work view helpers: where an item opens, and a workflow run as a project
 * (timeline, what's coming up, and when it should finish).
 */
import type { WorkItem, WorkflowRun, WorkflowStepRun } from '@/lib/bindings'
import type { View } from '@/store/navigation-store'

/** Assumed per step until some have finished */
export const DEFAULT_STEP_MS = 2 * 60 * 1000

/**
 * Where a work item opens: workflow runs as a project (until task 14 makes
 * them Requests), delegated tasks on their Request page
 */
export function workItemView(item: WorkItem): View {
  return item.kind === 'workflow'
    ? { kind: 'work-item', id: item.id }
    : { kind: 'request', requestId: item.id }
}

/** The run controls a workflow project offers */
export interface RunControls {
  pause: boolean
  resume: boolean
  cancel: boolean
}

/**
 * Mirrors what the backend accepts: pause a running run, resume only a
 * paused one (not one waiting on a person or an approval), cancel any run
 * that hasn't finished
 */
export function runControls(
  item: WorkItem,
  run: WorkflowRun | null
): RunControls {
  return {
    pause: item.state === 'active',
    resume: run?.status === 'paused',
    cancel: item.state === 'active' || item.state === 'waiting',
  }
}

/** A step ID as people read it: `draft-report` → "Draft report" */
export function stepLabel(stepId: string): string {
  const words = stepId.replace(/[-_]+/g, ' ').trim()
  return words.charAt(0).toUpperCase() + words.slice(1)
}

/** What a next action's sentence says, for `work.next.*` */
export interface NextActionMessage {
  key: 'approve' | 'manual' | 'paused' | 'working' | 'next'
  step: string
  agent: string
}

/**
 * The sentence under a run's progress, from the step and agent it names.
 * The backend's `description` carries IDs and is for Details only; a manual
 * step's own instructions are shown as written.
 */
export function nextActionMessage(
  run: WorkflowRun | null,
  next: NonNullable<WorkItem['nextAction']>,
  agentName: (id: string) => string
): NextActionMessage | { text: string } {
  const step = run?.steps.find(s => s.stepId === next.target?.stepId)
  const names = {
    step: step ? stepLabel(step.stepId) : '',
    agent: step ? agentName(step.agentId) : '',
  }
  if (next.kind === 'approval') return { key: 'approve', ...names }
  if (next.kind === 'manualStep') {
    if (!step) return { key: 'paused', ...names }
    return step.instructions
      ? { text: step.instructions }
      : { key: 'manual', ...names }
  }
  return { key: step?.status === 'running' ? 'working' : 'next', ...names }
}

export type TimelineKind =
  | 'runStarted'
  | 'stepStarted'
  | 'stepDone'
  | 'stepFailed'
  | 'stepRejected'
  | 'runEnded'

export interface TimelineEvent {
  at: number
  kind: TimelineKind
  stepId?: string
  agentId?: string
  /** The step's output or error, for finished steps */
  detail?: string | null
}

/** What happened, oldest first */
export function timeline(run: WorkflowRun): TimelineEvent[] {
  const events: TimelineEvent[] = [{ at: run.startedAt, kind: 'runStarted' }]
  for (const step of run.steps) {
    const base = { stepId: step.stepId, agentId: step.agentId }
    if (step.startedAt !== null) {
      events.push({ ...base, at: step.startedAt, kind: 'stepStarted' })
    }
    if (step.endedAt !== null) {
      const kind: TimelineKind =
        step.status === 'done'
          ? 'stepDone'
          : step.status === 'rejected'
            ? 'stepRejected'
            : 'stepFailed'
      events.push({
        ...base,
        at: step.endedAt,
        kind,
        detail: step.status === 'done' ? step.output : step.error,
      })
    }
  }
  if (run.endedAt !== null) {
    events.push({ at: run.endedAt, kind: 'runEnded', detail: run.error })
  }
  // Stable: a step's start before its end at the same instant
  return events
    .map((event, index) => ({ event, index }))
    .sort((a, b) => a.event.at - b.event.at || a.index - b.index)
    .map(({ event }) => event)
}

/** Mean duration of the steps that finished, or the default */
export function averageStepMs(run: WorkflowRun): number {
  const durations = run.steps
    .filter(s => s.startedAt !== null && s.endedAt !== null)
    .map(s => (s.endedAt ?? 0) - (s.startedAt ?? 0))
  if (durations.length === 0) return DEFAULT_STEP_MS
  return durations.reduce((sum, d) => sum + d, 0) / durations.length
}

export interface UpcomingStep {
  step: WorkflowStepRun
  /** When it should start, if nothing waits on you first */
  expectedAt: number
  needsApproval: boolean
}

const STOPPED = new Set(['completed', 'failed', 'cancelled'])

/** Steps not started yet, with when each should start */
export function comingUp(run: WorkflowRun, now: number): UpcomingStep[] {
  if (STOPPED.has(run.status)) return []
  const average = averageStepMs(run)
  const running = run.steps.find(s => s.status === 'running')
  let cursor =
    running?.startedAt != null
      ? Math.max(now, running.startedAt + average)
      : now
  const upcoming: UpcomingStep[] = []
  for (const step of run.steps) {
    if (step.status !== 'pending' && step.status !== 'awaitingApproval') {
      continue
    }
    upcoming.push({
      step,
      expectedAt: cursor,
      needsApproval: step.requiresApproval && !step.approved,
    })
    cursor += average
  }
  return upcoming
}

/** When the run should finish, or `null` once it's over */
export function estimatedFinish(run: WorkflowRun, now: number): number | null {
  if (STOPPED.has(run.status)) return null
  const upcoming = comingUp(run, now)
  const last = upcoming[upcoming.length - 1]
  if (last) return last.expectedAt + averageStepMs(run)
  const running = run.steps.find(s => s.status === 'running')
  if (running?.startedAt != null) {
    return Math.max(now, running.startedAt + averageStepMs(run))
  }
  return null
}
